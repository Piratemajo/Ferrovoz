//! Relé de voz: recibe las tramas cifradas de cada jugador y las reenvía,
//! cifradas con la clave de cada oyente:
//!
//! - Proximidad: misma dimensión y dentro del alcance (susurro, normal o grito).
//! - Grupo: solo a los miembros, a cualquier distancia y dimensión.
//!
//! Cada jugador va por UDP o, si no es posible, por el túnel (los paquetes viajan
//! en la conexión del juego; Java los pasa con `handle_tunnel` / `poll_tunnel`).
//!
//! Anti-replay por contador en el nonce y límite de tráfico por jugador e IP.
//! Los silencios de moderación y los grupos permanentes se guardan en un fichero.
//!
//! No decodifica audio: reenvía el Opus tal cual, así que gasta muy poca CPU. La
//! excepción son las multitudes (ver [`crate::crowd`]): si a una zona le llegan
//! más de `max_voices` voces, las lejanas se mezclan en el servidor.
//!
//! Aquí está el relé en sí (tipos, arranque, recibir y reenviar la voz). El resto:
//! `groups`, canales y escucha del micro en `audio`, multitudes en `crowds`,
//! paredes en `walls`, eventos en `events` y el archivo de datos en `storage`.

use crate::protocol::{
    self, AudioDown, AudioUp, Environment, GroupEntry, GroupList, GroupState, PacketType, PeerStates, PlayerId, PlayerKey,
    PongInfo, ReplayWindow,
    TalkMode, CROWD_ID, MAX_PACKET, OCCLUSION_UNKNOWN,
};
use crate::channels::{self, Channel, Target};
use crate::crowd::{self, Cell};
use crate::password::PasswordHash;
use crate::sync::lock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::fanout::{Body, Pool, SealJob};
use crate::roles::{Role, RoleRules};
use std::time::{Duration, Instant};

mod audio;
mod crowds;
mod events;
mod groups;
mod storage;
#[cfg(test)]
mod tests;
mod walls;

pub use events::{EventKind, MicFrame, ServerEvent};
pub use groups::GroupError;
use storage::Saver;

/// Máximo de paquetes en cola por jugador en el túnel (~1 s con 2-3 hablantes).
const TUNNEL_QUEUE_MAX: usize = 150;
/// Tramas por ventana de cálculo de pérdida.
const LOSS_WINDOW: u32 = 200;
pub const MAX_GROUP_NAME: usize = 24;
pub const MAX_PASSWORD: usize = 32;
/// Margen para considerar que alguien habla (para la oclusión).
const ACTIVE_SPEAKER: Duration = Duration::from_millis(400);
/// Parejas para calcular paredes como mucho: las más cercanas. Las demás se quedan
/// con la oclusión que calcula el cliente.
const ACTIVE_PAIRS_MAX: usize = 2000;
/// Validez de una oclusión calculada por el servidor.
const OCCLUSION_TTL: Duration = Duration::from_secs(1);
/// Cada cuánto se decide qué voces van por separado en las zonas con multitud.
const CROWD_PLAN_EVERY: u32 = 5;
/// Una trama de voz.
const FRAME: Duration = Duration::from_millis(20);
/// Distancia a la que se coloca el murmullo para el cliente: dentro de su zona de
/// volumen completo, porque la mezcla ya lleva la atenuación de cada voz.
const CROWD_REL_DISTANCE: f64 = 3.0;
/// Máximo de eventos en cola. Java los recoge cada tick; con 250 personas hablando
/// salen unos pocos por tick.
const EVENTS_MAX: usize = 4096;

/// Tramas del micro en cola para la API (unos 300 ms con 250 personas hablando).
const MIC_QUEUE_MAX: usize = 4096;
/// Alcance propio de un jugador como mucho (megáfono, escenario).
const MAX_RANGE_OVERRIDE: f32 = channels::MAX_RANGE;

// Límites de tráfico. Un hablante envía 50 tramas/s.
/// Intervalo de envío de los estados de voz.
const PEER_STATES_EVERY: Duration = Duration::from_secs(1);
pub(crate) const AUDIO_RATE: f32 = 60.0;
pub(crate) const AUDIO_BURST: f32 = 100.0;
const CONTROL_RATE: f32 = 5.0;
/// Cada cuánto como mucho se contesta a un `GroupsPlease`. La pantalla de ajustes
/// la pide una vez por segundo; el resto se descarta sin armar la lista.
const GROUP_LIST_EVERY: Duration = Duration::from_millis(400);
const CONTROL_BURST: f32 = 20.0;
/// Paquetes inválidos por segundo tolerados por IP antes de ignorarla.
/// Direcciones de las que se recuerda cuánta basura mandan.
const BAD_IPS_MAX: usize = 4096;
const BAD_PACKETS_RATE: f32 = 20.0;
const BAD_PACKETS_BURST: f32 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ranges {
    pub normal: f32,
    pub whisper: f32,
    pub shout: f32,
}

impl Default for Ranges {
    fn default() -> Self {
        Self { normal: 48.0, whisper: 8.0, shout: 96.0 }
    }
}

/// Dónde está un jugador en este tick (ver [`VoiceServer::update_players`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerUpdate {
    pub id: PlayerId,
    pub pos: [f64; 3],
    pub dimension: i32,
    pub underwater: bool,
    pub role: Role,
}

/// Token bucket: `rate` por segundo, máximo `burst`.
#[derive(Debug, Clone)]
pub(crate) struct TokenBucket {
    tokens: f32,
    last: Instant,
    rate: f32,
    burst: f32,
}

impl TokenBucket {
    pub(crate) fn new(rate: f32, burst: f32) -> Self {
        Self { tokens: burst, last: Instant::now(), rate, burst }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.last).as_secs_f32() * self.rate).min(self.burst);
        self.last = now;
    }

    /// Ya ha recuperado todo su crédito: no le queda castigo que recordar.
    fn full(&mut self) -> bool {
        self.refill();
        self.tokens >= self.burst
    }

    pub(crate) fn take(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn is_empty(&mut self) -> bool {
        self.refill();
        self.tokens < 1.0
    }
}

enum Transport {
    /// Sin Hello todavía.
    Pending,
    Udp(SocketAddr),
    Tunnel(VecDeque<Vec<u8>>),
}

#[derive(Default)]
struct LossMeter {
    last_seq: Option<u32>,
    expected: u32,
    received: u32,
    percent: u8,
}

impl LossMeter {
    fn record(&mut self, seq: u32) {
        match self.last_seq {
            Some(last) => {
                let gap = seq.wrapping_sub(last);
                // Duplicados, desordenados o saltos raros no cuentan.
                if gap == 0 || gap > 1000 {
                    if gap > 1000 {
                        self.last_seq = Some(seq);
                    }
                    return;
                }
                self.expected += gap;
                self.received += 1;
                self.last_seq = Some(seq);
            }
            None => self.last_seq = Some(seq),
        }
        if self.expected >= LOSS_WINDOW {
            let lost = self.expected.saturating_sub(self.received);
            self.percent = ((lost * 100) / self.expected).min(100) as u8;
            self.expected = 0;
            self.received = 0;
        }
    }
}

struct Player {
    key: PlayerKey,
    transport: Transport,
    rx_window: ReplayWindow,
    tx_counter: u64,
    audio_bucket: TokenBucket,
    control_bucket: TokenBucket,
    /// Última lista de grupos que se le mandó (ver [`GROUP_LIST_EVERY`]).
    groups_sent: Instant,
    /// Ya se avisó de una posición no numérica suya: no se repite 20 veces por segundo.
    warned_position: bool,
    pos: [f64; 3],
    dimension: i32,
    /// Zona de multitudes de `pos`; se recalcula solo al moverse (ver [`Player::place`]).
    cell: Cell,
    /// Cabeza bajo el agua.
    underwater: bool,
    /// Vivo, espectador o muerto (ver [`crate::roles`]).
    role: Role,
    /// Flags del cliente: mute (1) y ensordecido (2).
    self_flags: u8,
    can_speak: bool,
    can_shout: bool,
    group: Option<u32>,
    loss: LossMeter,
    env: Environment,
    last_audio: Option<Instant>,
    talk_range: f32,
    /// Se le oye siempre por separado, aunque haya multitud (`ferrovoz.priority`).
    priority: bool,
    /// Alcance fijado por un plugin en vez del de susurro, normal o grito.
    range_override: Option<f32>,
    /// Se le está oyendo (para los eventos de empezar y dejar de hablar).
    speaking: bool,
}

impl Player {
    fn place(&mut self, pos: [f64; 3], dimension: i32) {
        // Minecraft manda a veces una posición con NaN o infinito. Guardarla dejaba a ese
        // jugador sin oír ni ser oído y sin decir nada, porque toda comparación de
        // distancia con NaN es falsa. Mejor quedarse donde estaba.
        if !pos.iter().all(|c| c.is_finite()) {
            if !self.warned_position {
                self.warned_position = true;
                log::warn!("voice relay: ignoring a position that is not a number ({pos:?})");
            }
            return;
        }
        self.pos = pos;
        self.dimension = dimension;
        self.cell = Cell::of(pos, dimension);
    }

    fn seal(&mut self, kind: PacketType, body: &[u8]) -> Vec<u8> {
        self.tx_counter += 1;
        protocol::seal(&self.key, kind, None, body, self.tx_counter)
    }
}

struct Group {
    name: String,
    password: Option<PasswordHash>,
    owner: Option<PlayerId>,
    /// Permanente: no se borra al vaciarse, se guarda en disco y recuerda a sus miembros.
    persistent: bool,
    /// Miembros guardados (solo permanentes); al entrar se les vuelve a meter.
    saved_members: HashSet<PlayerId>,
}

#[derive(Default)]
pub struct ServerStats {
    pub packets_in: AtomicU64,
    pub packets_out: AtomicU64,
    pub rejected: AtomicU64,
    pub replayed: AtomicU64,
    pub rate_limited: AtomicU64,
    /// Tiempo (ns) de trabajo de los hilos del relé, incluidos los auxiliares.
    /// Dividido entre el tiempo real da el uso de CPU (100 % = un núcleo).
    pub busy_ns: AtomicU64,
    /// Lo mismo, solo de los hilos que reciben. Si se acerca al 100 % por hilo, no dan
    /// abasto y se pierden paquetes a la entrada.
    pub receive_busy_ns: AtomicU64,
}

struct State {
    players: HashMap<PlayerId, Player>,
    groups: HashMap<u32, Group>,
    next_group: u32,
    /// Silenciados por un admin. Fuera de `Player` para que se mantenga al reconectar
    /// (y se guarda en disco).
    admin_muted: HashSet<PlayerId>,
    ranges: Ranges,
    data_file: Option<PathBuf>,
    /// Quien escribe `data_file` (en su hilo, sin el lock).
    saver: Arc<Saver>,
    events: VecDeque<ServerEvent>,
    /// Oclusión (hablante, oyente) calculada por Java en el servidor.
    occlusion: HashMap<(PlayerId, PlayerId), (u8, Instant)>,
    bad_ips: HashMap<IpAddr, TokenBucket>,
    /// Reglas para espectadores y muertos.
    role_rules: RoleRules,
    /// Voces por separado como mucho en cada zona; 0 = sin límite.
    max_voices: usize,
    /// Zonas con multitud: qué voces van por separado.
    crowd_plans: HashMap<Cell, HashSet<PlayerId>>,
    /// Tramas que esperan a mezclarse en el murmullo.
    crowd_stash: crowd::Stash,
    /// Eco de cuevas y salas permitido. Si no, la voz se reenvía sin el sitio del
    /// hablante y el Pong avisa a los clientes para que apaguen también el suyo.
    cave_echo: bool,
    /// Efecto bajo el agua permitido. Si no, la voz se reenvía como si nadie estuviera
    /// sumergido y el Pong avisa a los clientes, como con el eco.
    underwater_effect: bool,
    /// Silencio general: solo se oye a quien tenga prioridad.
    hush: bool,
    /// Sonido de los plugins (ver [`crate::channels`]).
    channels: HashMap<PlayerId, Channel>,
    /// Algún plugin escucha la voz: las tramas se copian a `mic_frames`.
    mic_tap: bool,
    mic_frames: VecDeque<MicFrame>,
}

struct Shared {
    socket: UdpSocket,
    state: Mutex<State>,
    running: AtomicBool,
    stats: ServerStats,
}

pub struct VoiceServer {
    shared: Arc<Shared>,
    /// Los hilos que reciben (ver [`receiver_threads`]).
    threads: Vec<JoinHandle<()>>,
    crowd_thread: Option<JoinHandle<()>>,
    saver: Arc<Saver>,
    saver_thread: Option<JoinHandle<()>>,
}

/// Hilos que reciben. Por defecto uno: medido en un servidor de 20 núcleos con 150
/// personas hablando juntas, un hilo recibe sobrado (son unos 7 500 paquetes por
/// segundo) y con cuatro el relé gastaba un 26 % más de CPU para hacer lo mismo. Lo
/// caro (descifrar, copiar y cifrar para cada oyente) ya va fuera del lock y en el
/// pool. `FERROVOZ_RELAY_THREADS` pone más, para casos con muchísima voz de entrada.
fn receiver_threads() -> usize {
    std::env::var("FERROVOZ_RELAY_THREADS").ok().and_then(|v| v.trim().parse::<usize>().ok()).map_or(1, |n| n.clamp(1, 16))
}

/// Paquetes UDP pendientes de enviar.
type Outgoing = Vec<(SocketAddr, Vec<u8>)>;

/// Búferes del sistema para el socket del relé. Los de por defecto (64 KB en Windows)
/// se llenan en una ráfaga de unos cientos de tramas, por ejemplo cuando 300 personas
/// hablan a la vez y el hilo que recibe se para un momento; lo que no cabe se pierde.
const SOCKET_BUFFER: usize = 4 << 20;

fn bind_udp(bind: SocketAddr) -> io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(bind), Type::DGRAM, Some(Protocol::UDP))?;
    // Si el sistema da menos, se sigue con lo que dé.
    let _ = socket.set_recv_buffer_size(SOCKET_BUFFER);
    let _ = socket.set_send_buffer_size(SOCKET_BUFFER);
    socket.bind(&bind.into())?;
    if let (Ok(r), Ok(w)) = (socket.recv_buffer_size(), socket.send_buffer_size()) {
        log::info!("voice relay: UDP buffers {} KB in / {} KB out", r / 1024, w / 1024);
    }
    Ok(socket.into())
}

fn valid_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(|c| c.is_control())
}

impl VoiceServer {
    pub fn start(bind: SocketAddr, max_distance: f32) -> io::Result<Self> {
        Self::start_with_receivers(bind, max_distance, receiver_threads())
    }

    /// Como [`Self::start`], con un número de hilos que reciben a mano.
    pub fn start_with_receivers(bind: SocketAddr, max_distance: f32, receivers: usize) -> io::Result<Self> {
        let receivers = receivers.clamp(1, 16);
        let socket = bind_udp(bind)?;
        let saver = Arc::new(Saver::default());
        let saver_thread = {
            let saver = saver.clone();
            std::thread::Builder::new().name("ferrovoz-save".into()).spawn(move || saver.run())?
        };
        socket.set_read_timeout(Some(Duration::from_millis(250)))?;
        let shared = Arc::new(Shared {
            socket,
            state: Mutex::new(State {
                players: HashMap::new(),
                groups: HashMap::new(),
                next_group: 1,
                admin_muted: HashSet::new(),
                ranges: Ranges { normal: max_distance, ..Ranges::default() },
                data_file: None,
                saver: saver.clone(),
                events: VecDeque::new(),
                occlusion: HashMap::new(),
                bad_ips: HashMap::new(),
                role_rules: RoleRules::default(),
                max_voices: 0,
                crowd_plans: HashMap::new(),
                crowd_stash: crowd::Stash::default(),
                cave_echo: true,
                underwater_effect: true,
                hush: false,
                channels: HashMap::new(),
                mic_tap: false,
                mic_frames: VecDeque::new(),
            }),
            running: AtomicBool::new(true),
            stats: ServerStats::default(),
        });
        // Un pool de envío para todos los hilos que reciben. Vive mientras viva alguno.
        let stats = shared.clone();
        let on_sent: Arc<dyn Fn(usize, u64) + Send + Sync> = Arc::new(move |sent, busy| {
            stats.stats.packets_out.fetch_add(sent as u64, Ordering::Relaxed);
            stats.stats.busy_ns.fetch_add(busy, Ordering::Relaxed);
        });
        let pool = match Pool::new(&shared.socket, Pool::default_workers(receivers), on_sent.clone()) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("voice relay: could not start helper threads ({e}), using fewer");
                Pool::new(&shared.socket, 0, on_sent)?
            }
        };
        log::info!("voice relay: {receivers} receiving and {} sending thread(s)", pool.workers());
        let pool = Arc::new(pool);
        let mut threads = Vec::with_capacity(receivers);
        for i in 0..receivers {
            let (worker, pool) = (shared.clone(), pool.clone());
            let name = if i == 0 { "ferrovoz-server".to_owned() } else { format!("ferrovoz-server-{i}") };
            let spawned = std::thread::Builder::new().name(name).spawn(move || worker.run(i, &pool));
            match spawned {
                Ok(t) => threads.push(t),
                Err(e) if i > 0 => {
                    log::warn!("voice relay: could not start receiving thread {i} ({e})");
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        drop(pool);
        let mixer = shared.clone();
        let crowd_thread = std::thread::Builder::new()
            .name("ferrovoz-crowd".into())
            .spawn(move || mixer.crowd_loop())?;
        Ok(Self { shared, threads, crowd_thread: Some(crowd_thread), saver, saver_thread: Some(saver_thread) })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.shared.socket.local_addr()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.shared.state)
    }

    /// Ejecuta `f` con el estado y después envía lo pendiente por UDP.
    fn with_state<T>(&self, f: impl FnOnce(&mut State, &mut Outgoing) -> T) -> T {
        let mut out = Vec::new();
        let r = {
            let mut st = self.state();
            f(&mut st, &mut out)
        };
        self.shared.send_udp(out);
        r
    }

    // Jugadores

    /// Registra (o vuelve a registrar) a un jugador y devuelve su clave nueva.
    pub fn add_player(&self, id: PlayerId) -> PlayerKey {
        let key = protocol::random_key();
        self.with_state(|st, out| {
            if let Some(old) = st.players.remove(&id) {
                st.note_gone(&id, &old);
                if let Some(g) = old.group {
                    st.cleanup_group(g);
                }
            }
            st.players.insert(
                id,
                Player {
                    key,
                    transport: Transport::Pending,
                    rx_window: ReplayWindow::default(),
                    tx_counter: 0,
                    audio_bucket: TokenBucket::new(AUDIO_RATE, AUDIO_BURST),
                    control_bucket: TokenBucket::new(CONTROL_RATE, CONTROL_BURST),
                    groups_sent: Instant::now() - GROUP_LIST_EVERY,
                    warned_position: false,
                    pos: [0.0; 3],
                    dimension: 0,
                    cell: Cell::of([0.0; 3], 0),
                    underwater: false,
                    role: Role::Alive,
                    self_flags: 0,
                    can_speak: true,
                    can_shout: true,
                    group: None,
                    loss: LossMeter::default(),
                    env: Environment::default(),
                    last_audio: None,
                    talk_range: 0.0,
                    priority: false,
                    range_override: None,
                    speaking: false,
                },
            );
            // Si era miembro de un grupo permanente, vuelve a él.
            let saved = st.groups.iter().find(|(_, g)| g.persistent && g.saved_members.contains(&id)).map(|(gid, _)| *gid);
            if let Some(gid) = saved {
                st.set_group(&id, Some(gid), out);
                let name = st.groups[&gid].name.clone();
                st.push_event(EventKind::Rejoined, name, id);
            }
        });
        key
    }

    pub fn remove_player(&self, id: &PlayerId) {
        self.with_state(|st, out| {
            if let Some(p) = st.players.remove(id) {
                st.note_gone(id, &p);
                if let Some(g) = p.group {
                    let name = st.groups.get(&g).map(|g| g.name.clone()).unwrap_or_default();
                    st.push_event(EventKind::Left, name, *id);
                    st.after_member_left(g, id, out);
                }
            }
            st.occlusion.retain(|(a, b), _| a != id && b != id);
        });
    }

    pub fn update_player(&self, id: &PlayerId, pos: [f64; 3], dimension: i32) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.place(pos, dimension);
        }
    }

    /// Como [`Self::update_player`], más si tiene la cabeza bajo el agua.
    pub fn update_player_full(&self, id: &PlayerId, pos: [f64; 3], dimension: i32, underwater: bool) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.place(pos, dimension);
            p.underwater = underwater;
        }
    }

    /// Como [`Self::update_player_full`], más el rol (vivo, espectador, muerto).
    pub fn update_player_with_role(&self, id: &PlayerId, pos: [f64; 3], dimension: i32, underwater: bool, role: Role) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.place(pos, dimension);
            p.underwater = underwater;
            p.role = role;
        }
    }

    /// Lo mismo que [`Self::update_player_with_role`] para muchos jugadores, cogiendo el
    /// candado una sola vez. Java lo llama una vez por tick con todos: así el hilo del
    /// juego no compite cientos de veces por tick con el que reenvía la voz.
    pub fn update_players(&self, updates: &[PlayerUpdate]) {
        let mut st = self.state();
        for u in updates {
            if let Some(p) = st.players.get_mut(&u.id) {
                p.place(u.pos, u.dimension);
                p.underwater = u.underwater;
                p.role = u.role;
            }
        }
    }

    /// Reglas para espectadores y muertos (ver [`crate::roles`]).
    pub fn set_role_rules(&self, rules: RoleRules) {
        self.state().role_rules = rules;
    }

    /// Silencio de moderación. Se mantiene al reconectar y tras reiniciar.
    pub fn set_muted(&self, id: &PlayerId, muted: bool) {
        let mut st = self.state();
        let changed = if muted { st.admin_muted.insert(*id) } else { st.admin_muted.remove(id) };
        if changed {
            st.save();
        }
    }

    pub fn is_muted(&self, id: &PlayerId) -> bool {
        self.state().admin_muted.contains(id)
    }

    /// Permisos (LuckPerms, op...): hablar y gritar.
    pub fn set_permissions(&self, id: &PlayerId, can_speak: bool, can_shout: bool) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.can_speak = can_speak;
            p.can_shout = can_shout;
        }
    }

    /// Su voz llega siempre por separado, aunque haya multitud (el anfitrión de un evento).
    pub fn set_priority(&self, id: &PlayerId, priority: bool) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.priority = priority;
        }
    }

    /// Voces por separado como mucho en cada zona (0 = sin límite).
    pub fn set_max_voices(&self, max: usize) {
        let mut st = self.state();
        st.max_voices = max;
        if max == 0 {
            st.crowd_plans.clear();
        }
    }

    /// Eco de cuevas y salas para todos (por defecto, sí). Los clientes se enteran
    /// en el siguiente Pong, como mucho en 2 s.
    /// Efecto bajo el agua para todos (por defecto, sí). Los clientes se enteran en el
    /// siguiente Pong.
    pub fn set_underwater_effect(&self, enabled: bool) {
        self.state().underwater_effect = enabled;
    }

    pub fn set_cave_echo(&self, enabled: bool) {
        self.state().cave_echo = enabled;
    }

    /// Alcance propio para un jugador (megáfono, escenario...) en vez del de su modo;
    /// `None` lo quita. Se pierde al volver a registrarle.
    pub fn set_range_override(&self, id: &PlayerId, range: Option<f32>) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.range_override = range.map(|r| if r.is_finite() { r.clamp(1.0, MAX_RANGE_OVERRIDE) } else { MAX_RANGE_OVERRIDE });
        }
    }

    /// Silencio general: no se oye a nadie salvo a quien tiene prioridad. Los demás
    /// salen con el micro silenciado en los iconos.
    pub fn set_hush(&self, hush: bool) {
        self.state().hush = hush;
    }

    pub fn is_hushed(&self) -> bool {
        self.state().hush
    }

    /// Se le está oyendo ahora.
    pub fn is_speaking(&self, id: &PlayerId) -> bool {
        self.state().players.get(id).is_some_and(|p| p.speaking)
    }

    /// Ya le llega la voz (ha hecho el Hello y no se ha ido).
    pub fn is_connected(&self, id: &PlayerId) -> bool {
        self.state().players.get(id).is_some_and(|p| !matches!(p.transport, Transport::Pending))
    }

    /// Flags que manda su cliente: micro silenciado (1) y ensordecido (2).
    pub fn self_flags(&self, id: &PlayerId) -> Option<u8> {
        self.state().players.get(id).map(|p| p.self_flags)
    }

    /// Los jugadores registrados.
    pub fn player_ids(&self) -> Vec<PlayerId> {
        self.state().players.keys().copied().collect()
    }

    pub fn set_max_distance(&self, d: f32) {
        self.state().ranges.normal = d.max(1.0);
    }

    pub fn set_ranges(&self, r: Ranges) {
        self.state().ranges = Ranges { normal: r.normal.max(1.0), whisper: r.whisper.max(1.0), shout: r.shout.max(1.0) };
    }

    pub fn player_count(&self) -> usize {
        self.state().players.len()
    }

    pub fn connected_count(&self) -> usize {
        self.state().players.values().filter(|p| !matches!(p.transport, Transport::Pending)).count()
    }

    pub fn tunnel_count(&self) -> usize {
        self.state().players.values().filter(|p| matches!(p.transport, Transport::Tunnel(_))).count()
    }

    /// Pérdida (%) en la subida de un jugador.
    pub fn loss_percent(&self, id: &PlayerId) -> Option<u8> {
        self.state().players.get(id).map(|p| p.loss.percent)
    }

    pub fn stats(&self) -> &ServerStats {
        &self.shared.stats
    }

    // Túnel

    /// Paquete recibido por la conexión del juego.
    pub fn handle_tunnel(&self, from: &PlayerId, data: &[u8]) {
        self.shared.stats.packets_in.fetch_add(1, Ordering::Relaxed);
        let mut out = Vec::new();
        let mut jobs = Vec::new();
        if self.shared.handle(data, Source::Tunnel(*from), &mut out, &mut jobs).is_err() {
            self.shared.stats.rejected.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.send_udp(out);
        // Viene del hilo del juego: se cifra aquí, sin el pool.
        let sent = crate::fanout::seal_and_send(&self.shared.socket, jobs);
        self.shared.stats.packets_out.fetch_add(sent as u64, Ordering::Relaxed);
    }

    /// Paquetes pendientes para un jugador por túnel.
    pub fn poll_tunnel(&self, id: &PlayerId) -> Vec<Vec<u8>> {
        match self.state().players.get_mut(id).map(|p| &mut p.transport) {
            Some(Transport::Tunnel(q)) => q.drain(..).collect(),
            _ => Vec::new(),
        }
    }

    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        if let Some(t) = self.crowd_thread.take() {
            let _ = t.join();
        }
        // Lo último que haya cambiado queda en disco antes de volver.
        self.saver.close();
        if let Some(t) = self.saver_thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for VoiceServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Entrega un paquete a un jugador por su transporte.
fn deliver(stats: &ServerStats, player: &mut Player, pkt: Vec<u8>, out: &mut Outgoing) {
    match &mut player.transport {
        Transport::Udp(addr) => out.push((*addr, pkt)),
        Transport::Tunnel(q) => {
            if q.len() >= TUNNEL_QUEUE_MAX {
                q.pop_front(); // mejor perder lo viejo que acumular latencia
            }
            q.push_back(pkt);
            stats.packets_out.fetch_add(1, Ordering::Relaxed);
        }
        Transport::Pending => {}
    }
}

impl State {
}

#[derive(Clone, Copy)]
enum Source {
    Udp(SocketAddr),
    Tunnel(PlayerId),
}

impl Shared {
    /// Un hilo que recibe. Varios esperan en el mismo socket y el sistema da cada
    /// paquete a uno; lo caro (descifrar, copiar y cifrar para cada oyente) va fuera
    /// del lock, así que trabajan a la vez. El `index` 0 manda además los estados de
    /// voz.
    fn run(self: &Arc<Self>, index: usize, pool: &Pool) {
        let mut buf = [0u8; MAX_PACKET];
        let mut jobs = Vec::new();
        let mut last_states = Instant::now();
        while self.running.load(Ordering::Relaxed) {
            // Estados de voz a todos, cada segundo.
            if index == 0 && last_states.elapsed() >= PEER_STATES_EVERY {
                last_states = Instant::now();
                let mut out = Vec::new();
                self.broadcast_peer_states(&mut out);
                self.send_udp(out);
            }
            let (len, from) = match self.socket.recv_from(&mut buf) {
                Ok(v) => v,
                // Timeout, o en Windows un ICMP de puerto inalcanzable de un envío anterior.
                Err(_) => continue,
            };
            let started = Instant::now();
            self.stats.packets_in.fetch_add(1, Ordering::Relaxed);
            let mut out = Vec::new();
            // Un fallo con un paquete raro no se lleva el hilo por delante: se descarta.
            let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.handle(&buf[..len], Source::Udp(from), &mut out, &mut jobs)
            }));
            match handled {
                Ok(Ok(())) => self.send_udp(out),
                Ok(Err(())) => {
                    self.stats.rejected.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    log::error!("voice relay: failed on a packet from {from}; skipped it");
                    self.stats.rejected.fetch_add(1, Ordering::Relaxed);
                    jobs.clear();
                }
            }
            if !jobs.is_empty() {
                pool.dispatch(std::mem::take(&mut jobs));
            }
            let busy = started.elapsed().as_nanos() as u64;
            self.stats.busy_ns.fetch_add(busy, Ordering::Relaxed);
            self.stats.receive_busy_ns.fetch_add(busy, Ordering::Relaxed);
        }
    }

    fn broadcast_peer_states(&self, out: &mut Outgoing) {
        let mut st = lock(&self.state);
        let st = &mut *st;
        let entries: Vec<(PlayerId, u8)> = st
            .players
            .iter()
            .map(|(id, p)| {
                let mut flags = p.self_flags & (PeerStates::MUTED | PeerStates::DEAFENED);
                if !p.can_speak || st.admin_muted.contains(id) || (st.hush && !p.priority) {
                    flags |= PeerStates::MUTED;
                }
                if !matches!(p.transport, Transport::Pending) {
                    flags |= PeerStates::CONNECTED;
                }
                (*id, flags)
            })
            .collect();
        let bodies: Vec<Vec<u8>> = entries
            .chunks(PeerStates::PER_PACKET)
            .map(|c| PeerStates { entries: c.to_vec() }.encode())
            .collect();
        for p in st.players.values_mut() {
            if matches!(p.transport, Transport::Pending) {
                continue;
            }
            for body in &bodies {
                let pkt = p.seal(PacketType::PeerStates, body);
                deliver(&self.stats, p, pkt, out);
            }
        }
    }

    fn send_udp(&self, out: Outgoing) {
        // En Linux, en lotes con sendmmsg.
        let sent = crate::udpbatch::send_all(&self.socket, &out);
        self.stats.packets_out.fetch_add(sent as u64, Ordering::Relaxed);
    }

    /// IP que manda demasiada basura: se ignora un rato sin descifrar nada.
    fn note_bad_packet(st: &mut State, source: Source) {
        if let Source::Udp(addr) = source {
            if st.bad_ips.len() > BAD_IPS_MAX {
                // Solo se olvidan las que ya han recuperado su crédito. Vaciarlo entero
                // dejaba que una ráfaga desde muchas direcciones (o falsificándolas)
                // borrase también el castigo de quien estaba bloqueado.
                st.bad_ips.retain(|_, b| !b.full());
                if st.bad_ips.len() > BAD_IPS_MAX {
                    st.bad_ips.clear();
                }
            }
            st.bad_ips
                .entry(addr.ip())
                .or_insert_with(|| TokenBucket::new(BAD_PACKETS_RATE, BAD_PACKETS_BURST))
                .take();
        }
    }

    /// Un paquete de un cliente. En tres pasos para que varios hilos trabajen a la vez:
    /// bajo el lock lo barato (IP bloqueada, quién es, límite de tráfico, repetido),
    /// fuera el descifrado, y otra vez bajo el lock marcar el contador y aplicarlo.
    fn handle(&self, data: &[u8], source: Source, out: &mut Outgoing, jobs: &mut Vec<SealJob>) -> Result<(), ()> {
        let parsed = protocol::parse(data);

        // 1. Comprobaciones baratas.
        let (raw, sender_id, key, counter) = {
            let mut guard = lock(&self.state);
            let st = &mut *guard;
            if let Source::Udp(addr) = source {
                if st.bad_ips.get_mut(&addr.ip()).is_some_and(|b| b.is_empty()) {
                    self.stats.rate_limited.fetch_add(1, Ordering::Relaxed);
                    return Err(());
                }
            }
            let sender_id = parsed.as_ref().ok().and_then(|r| r.sender);
            let (Ok(raw), Some(sender_id)) = (parsed, sender_id) else {
                Self::note_bad_packet(st, source);
                return Err(());
            };
            if let Source::Tunnel(owner) = source {
                // Por el túnel solo puede hablar el dueño de esa conexión.
                if owner != sender_id {
                    return Err(());
                }
            }
            let Some(sender) = st.players.get_mut(&sender_id) else {
                Self::note_bad_packet(st, source);
                return Err(());
            };
            // Límite por jugador antes de descifrar, que es lo caro.
            let allowed = match raw.kind {
                PacketType::AudioUp => sender.audio_bucket.take(),
                _ => sender.control_bucket.take(),
            };
            if !allowed {
                self.stats.rate_limited.fetch_add(1, Ordering::Relaxed);
                return Err(());
            }
            let Ok(counter) = raw.counter() else {
                Self::note_bad_packet(st, source);
                return Err(());
            };
            if !sender.rx_window.check(counter) {
                self.stats.replayed.fetch_add(1, Ordering::Relaxed);
                return Err(());
            }
            (raw, sender_id, sender.key, counter)
        };

        // 2. Descifrar, sin lock.
        let Ok(body) = raw.open(&key) else {
            Self::note_bad_packet(&mut lock(&self.state), source);
            return Err(());
        };
        let frame = match raw.kind {
            PacketType::AudioUp => Some(AudioUp::decode(&body).map_err(|_| ())?),
            _ => None,
        };

        // 3. Aplicarlo.
        let mut guard = lock(&self.state);
        let st = &mut *guard;
        let (cave_echo, hush, underwater_effect) = (st.cave_echo, st.hush, st.underwater_effect);
        let Some(sender) = st.players.get_mut(&sender_id) else {
            return Err(()); // ha salido mientras
        };
        if sender.key != key {
            return Err(()); // se le ha vuelto a registrar con otra clave mientras
        }
        // Otro hilo puede haber aceptado el mismo paquete mientras se descifraba.
        if !sender.rx_window.check(counter) {
            self.stats.replayed.fetch_add(1, Ordering::Relaxed);
            return Err(());
        }
        sender.rx_window.mark(counter);

        match raw.kind {
            PacketType::Hello | PacketType::Ping => {
                // Solo estos paquetes fijan el transporte (cambio de puerto NAT, paso a
                // túnel). El anti-replay impide reenviar un Hello viejo para desviar la voz.
                let first = matches!(sender.transport, Transport::Pending);
                match source {
                    Source::Udp(addr) => sender.transport = Transport::Udp(addr),
                    Source::Tunnel(_) => {
                        if !matches!(sender.transport, Transport::Tunnel(_)) {
                            sender.transport = Transport::Tunnel(VecDeque::new());
                        }
                    }
                }
                let pong = PongInfo {
                    loss_percent: sender.loss.percent,
                    no_cave_echo: !cave_echo,
                    no_underwater: !underwater_effect,
                }
                .encode();
                let pkt = sender.seal(PacketType::Pong, &pong);
                deliver(&self.stats, sender, pkt, out);
                if first || raw.kind == PacketType::Hello {
                    st.send_group_state(&sender_id, out);
                }
                if first {
                    st.push_event(EventKind::Connected, String::new(), sender_id);
                }
            }
            PacketType::AudioUp => {
                let from_ok = match (&sender.transport, source) {
                    (Transport::Udp(a), Source::Udp(b)) => *a == b,
                    (Transport::Tunnel(_), Source::Tunnel(_)) => true,
                    _ => false,
                };
                if !from_ok {
                    return Err(());
                }
                let frame = frame.ok_or(())?;
                sender.loss.record(frame.seq);
                // Sin eco en el servidor, la voz sale sin el sitio del hablante: así
                // tampoco lo oyen los clientes antiguos.
                sender.env = if cave_echo { frame.env } else { Environment::default() };
                if !sender.can_speak || st.admin_muted.contains(&sender_id) || (hush && !sender.priority) {
                    return Ok(()); // silenciado: se acepta pero no se reenvía
                }
                let (src_pos, src_dim, group, env, underwater, src_role) =
                    (sender.pos, sender.dimension, sender.group, sender.env, sender.underwater && underwater_effect, sender.role);
                let rules = st.role_rules;
                if !rules.can_be_heard(src_role) {
                    return Ok(()); // p. ej. espectador con "listen"
                }
                let talk = match frame.talk {
                    TalkMode::Shout if !sender.can_shout => TalkMode::Normal,
                    t => t,
                };
                let range = sender.range_override.unwrap_or(match talk {
                    TalkMode::Normal => st.ranges.normal,
                    TalkMode::Whisper => st.ranges.whisper,
                    TalkMode::Shout => st.ranges.shout,
                });
                sender.last_audio = Some(Instant::now());
                sender.talk_range = range;
                let started = !std::mem::replace(&mut sender.speaking, true);
                let src_priority = sender.priority;
                if started {
                    let mode = if group.is_some() { "group" } else { talk.name() };
                    st.push_event(EventKind::StartedSpeaking, mode.to_owned(), sender_id);
                }
                if st.mic_tap {
                    if st.mic_frames.len() >= MIC_QUEUE_MAX {
                        st.mic_frames.pop_front(); // nadie las recoge a tiempo: mejor perder las viejas
                    }
                    let opus = frame.opus.clone();
                    st.mic_frames.push_back(MicFrame { player: sender_id, seq: frame.seq, talk, group: group.is_some(), opus });
                }

                // Quién la oye, en una sola vuelta. Con multitud, a las zonas donde esta
                // voz no va por separado le llega en el murmullo; el plan se mira una vez
                // por zona (unas pocas), no por oyente (cientos).
                let crowd_check = group.is_none() && !src_priority && !st.crowd_plans.is_empty();
                let mut zones: Vec<(Cell, bool)> = Vec::new();
                let mut to_crowd = false;
                let range2 = (range as f64) * (range as f64);
                let mut recipients: Vec<PlayerId> = Vec::new();
                for (id, l) in &st.players {
                    if *id == sender_id || matches!(l.transport, Transport::Pending) || !rules.can_hear(src_role, l.role) {
                        continue;
                    }
                    match group {
                        Some(g) => {
                            if l.group != Some(g) {
                                continue;
                            }
                        }
                        None => {
                            if l.dimension != src_dim {
                                continue;
                            }
                            let d = [src_pos[0] - l.pos[0], src_pos[1] - l.pos[1], src_pos[2] - l.pos[2]];
                            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > range2 {
                                continue;
                            }
                            if crowd_check {
                                let separate = match zones.iter().find(|(c, _)| *c == l.cell) {
                                    Some((_, v)) => *v,
                                    None => {
                                        let v = st.crowd_plans.get(&l.cell).is_none_or(|near| near.contains(&sender_id));
                                        zones.push((l.cell, v));
                                        v
                                    }
                                };
                                if !separate {
                                    to_crowd = true;
                                    continue;
                                }
                            }
                        }
                    }
                    recipients.push(*id);
                }
                if to_crowd && src_role == Role::Alive {
                    let speaker = crowd::Speaker { id: sender_id, pos: src_pos, dim: src_dim, range };
                    st.crowd_stash.push(speaker, frame.opus.clone(), Instant::now());
                }

                let now = Instant::now();
                // La trama se monta una vez; para cada oyente solo cambian su
                // posición relativa y la oclusión.
                let template = Arc::new(AudioDown {
                    sender: sender_id,
                    seq: frame.seq,
                    rel: [0.0; 3],
                    range: if group.is_some() { 0.0 } else { range },
                    group: group.is_some(),
                    underwater,
                    env,
                    occlusion: OCCLUSION_UNKNOWN,
                    opus: frame.opus,
                }
                .encode());
                for id in recipients {
                    let occlusion = match st.occlusion.get(&(sender_id, id)) {
                        Some((v, t)) if now.duration_since(*t) < OCCLUSION_TTL => *v,
                        _ => OCCLUSION_UNKNOWN,
                    };
                    let listener = st.players.get_mut(&id).unwrap();
                    let rel = [src_pos[0] - listener.pos[0], src_pos[1] - listener.pos[1], src_pos[2] - listener.pos[2]];
                    let rel = [rel[0] as f32, rel[1] as f32, rel[2] as f32];
                    match listener.transport {
                        // UDP: el contador se reserva aquí; la copia y el cifrado, fuera del lock.
                        Transport::Udp(addr) => {
                            listener.tx_counter += 1;
                            let body = Body::Audio { template: template.clone(), rel, occlusion };
                            jobs.push(SealJob { addr, key: listener.key, counter: listener.tx_counter, body });
                        }
                        _ => {
                            let body = AudioDown::for_listener(&template, rel, occlusion);
                            let pkt = listener.seal(PacketType::AudioDown, &body);
                            deliver(&self.stats, listener, pkt, out);
                        }
                    }
                }
            }
            PacketType::VoiceState => {
                sender.self_flags = body.first().copied().unwrap_or(0) & (PeerStates::MUTED | PeerStates::DEAFENED);
            }
            PacketType::GroupsPlease => {
                if sender.groups_sent.elapsed() >= GROUP_LIST_EVERY {
                    sender.groups_sent = Instant::now();
                    st.send_group_list(&sender_id, out);
                }
            }
            PacketType::Bye => {
                // Solo desde su dirección o su túnel (además va cifrado con su clave).
                let from_ok = match (&sender.transport, source) {
                    (Transport::Udp(a), Source::Udp(b)) => *a == b,
                    (Transport::Tunnel(_), Source::Tunnel(_)) => true,
                    _ => false,
                };
                if !from_ok {
                    return Err(());
                }
                sender.transport = Transport::Pending;
                let was_speaking = std::mem::replace(&mut sender.speaking, false);
                if was_speaking {
                    st.push_event(EventKind::StoppedSpeaking, String::new(), sender_id);
                }
                st.push_event(EventKind::Disconnected, String::new(), sender_id);
            }
            // Esto no lo manda nunca un cliente.
            PacketType::AudioDown
            | PacketType::Pong
            | PacketType::GroupState
            | PacketType::RelayAudio
            | PacketType::RelayGroup
            | PacketType::PeerStates
            | PacketType::GroupList => return Err(()),
        }
        Ok(())
    }
}
