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

use crate::protocol::{
    self, AudioDown, AudioUp, Environment, GroupState, PacketType, PeerStates, PlayerId, PlayerKey, PongInfo, ReplayWindow,
    TalkMode, CROWD_ID, MAX_PACKET, OCCLUSION_UNKNOWN,
};
use crate::channels::{self, Channel, Target};
use crate::crowd::{self, Cell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::fanout::{Pool, SealJob};
use crate::roles::{Role, RoleRules};
use std::time::{Duration, Instant};

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
const CONTROL_BURST: f32 = 20.0;
/// Paquetes inválidos por segundo tolerados por IP antes de ignorarla.
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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum GroupError {
    InvalidName,
    AlreadyExists,
    NotFound,
    WrongPassword,
    UnknownPlayer,
    NotOwner,
    NotInGroup,
}

impl GroupError {
    /// Código numérico para Java.
    pub fn code(self) -> i32 {
        match self {
            Self::InvalidName => 1,
            Self::AlreadyExists => 2,
            Self::NotFound => 3,
            Self::WrongPassword => 4,
            Self::UnknownPlayer => 5,
            Self::NotOwner => 6,
            Self::NotInGroup => 7,
        }
    }
}

/// Algo que ha pasado en el relé y Java tiene que saber: avisos de grupo por chat y
/// eventos de la API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEvent {
    pub kind: EventKind,
    pub player: PlayerId,
    /// Grupo, o el modo al empezar a hablar (`normal`, `whisper`, `shout`, `group`).
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Joined,
    Left,
    Kicked,
    /// Ha vuelto a entrar y se le ha devuelto a su grupo permanente.
    Rejoined,
    NewOwner,
    /// `player` es quien lo borró, o todo ceros si fue un plugin.
    Deleted,
    /// Primer Hello: ya le llega la voz.
    Connected,
    /// Ha dejado la voz (Bye o salió del servidor).
    Disconnected,
    /// Empieza a sonar su voz.
    StartedSpeaking,
    /// Lleva [`ACTIVE_SPEAKER`] sin sonar.
    StoppedSpeaking,
}

impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Joined => "join",
            Self::Left => "leave",
            Self::Kicked => "kick",
            Self::Rejoined => "rejoin",
            Self::NewOwner => "owner",
            Self::Deleted => "delete",
            Self::Connected => "connect",
            Self::Disconnected => "disconnect",
            Self::StartedSpeaking => "speak",
            Self::StoppedSpeaking => "silent",
        }
    }
}

/// Trama del micro de un jugador, para los plugins que escuchan la voz (API).
#[derive(Debug, Clone, PartialEq)]
pub struct MicFrame {
    pub player: PlayerId,
    pub seq: u32,
    pub talk: TalkMode,
    /// Habla a su grupo, no por proximidad.
    pub group: bool,
    pub opus: Vec<u8>,
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
    pos: [f64; 3],
    dimension: i32,
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
    fn seal(&mut self, kind: PacketType, body: &[u8]) -> Vec<u8> {
        self.tx_counter += 1;
        protocol::seal(&self.key, kind, None, body, self.tx_counter)
    }
}

struct Group {
    name: String,
    password: Option<String>,
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
    thread: Option<JoinHandle<()>>,
    crowd_thread: Option<JoinHandle<()>>,
}

/// Paquetes UDP pendientes de enviar.
type Outgoing = Vec<(SocketAddr, Vec<u8>)>;

fn valid_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(|c| c.is_control())
}

impl VoiceServer {
    pub fn start(bind: SocketAddr, max_distance: f32) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
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
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("ferrovoz-server".into())
            .spawn(move || worker.run())?;
        let mixer = shared.clone();
        let crowd_thread = std::thread::Builder::new()
            .name("ferrovoz-crowd".into())
            .spawn(move || mixer.crowd_loop())?;
        Ok(Self { shared, thread: Some(thread), crowd_thread: Some(crowd_thread) })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.shared.socket.local_addr()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state.lock().unwrap()
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

    // Persistencia

    /// Fichero de silencios y grupos permanentes. Lo carga si existe.
    pub fn set_data_file(&self, path: &Path) -> io::Result<()> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut st = self.state();
        st.data_file = Some(path.to_owned());
        st.load(&text);
        Ok(())
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
                    pos: [0.0; 3],
                    dimension: 0,
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
            p.pos = pos;
            p.dimension = dimension;
        }
    }

    /// Como [`Self::update_player`], más si tiene la cabeza bajo el agua.
    pub fn update_player_full(&self, id: &PlayerId, pos: [f64; 3], dimension: i32, underwater: bool) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.pos = pos;
            p.dimension = dimension;
            p.underwater = underwater;
        }
    }

    /// Como [`Self::update_player_full`], más el rol (vivo, espectador, muerto).
    pub fn update_player_with_role(&self, id: &PlayerId, pos: [f64; 3], dimension: i32, underwater: bool, role: Role) {
        if let Some(p) = self.state().players.get_mut(id) {
            p.pos = pos;
            p.dimension = dimension;
            p.underwater = underwater;
            p.role = role;
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

    // Escucha del micro (API)

    /// Copia (o deja de copiar) las tramas del micro para [`Self::poll_mic`]. Solo las
    /// que se reenvían: un silenciado no sale.
    pub fn set_mic_tap(&self, on: bool) {
        let mut st = self.state();
        st.mic_tap = on;
        if !on {
            st.mic_frames.clear();
        }
    }

    pub fn poll_mic(&self) -> Vec<MicFrame> {
        self.state().mic_frames.drain(..).collect()
    }

    // Canales de audio (API)

    /// Canal nuevo. `None` si ya hay [`channels::MAX_CHANNELS`] o Opus falla.
    pub fn create_channel(&self, target: Target, range: f32, bitrate: i32, music: bool) -> Option<PlayerId> {
        let channel = Channel::new(target, range, bitrate, music).ok()?;
        let mut st = self.state();
        if st.channels.len() >= channels::MAX_CHANNELS {
            return None;
        }
        let id = channels::new_id();
        st.channels.insert(id, channel);
        Some(id)
    }

    pub fn set_channel_target(&self, id: &PlayerId, target: Target) -> bool {
        self.state().channels.get_mut(id).map(|c| c.target = target).is_some()
    }

    pub fn set_channel_range(&self, id: &PlayerId, range: f32) -> bool {
        self.state().channels.get_mut(id).map(|c| c.range = channels::clamp_range(range)).is_some()
    }

    pub fn remove_channel(&self, id: &PlayerId) -> bool {
        self.state().channels.remove(id).is_some()
    }

    pub fn channel_count(&self) -> usize {
        self.state().channels.len()
    }

    /// Codifica 20 ms de audio (960 muestras mono, -1..1) y lo envía por el canal.
    /// Codifica en el hilo que llama, sin bloquear al relé.
    pub fn channel_send_pcm(&self, id: &PlayerId, pcm: &[f32]) -> bool {
        if pcm.len() != protocol::FRAME_SAMPLES {
            return false;
        }
        let Some(encoder) = self.state().channels.get(id).map(|c| c.encoder.clone()) else { return false };
        let opus = {
            let mut enc = encoder.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match enc.encode(pcm) {
                Ok(o) => o.to_vec(),
                Err(_) => return false,
            }
        };
        self.channel_send_opus(id, &opus)
    }

    /// Envía una trama Opus ya codificada (20 ms, 48 kHz, mono).
    pub fn channel_send_opus(&self, id: &PlayerId, opus: &[u8]) -> bool {
        if opus.is_empty() || opus.len() > crate::codec::MAX_OPUS_FRAME {
            return false;
        }
        let mut out = Vec::new();
        let mut jobs = Vec::new();
        let sent = self.shared.route_channel(id, opus, &mut out, &mut jobs);
        self.shared.send_udp(out);
        let n = crate::fanout::seal_and_send(&self.shared.socket, jobs);
        self.shared.stats.packets_out.fetch_add(n as u64, Ordering::Relaxed);
        sent
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

    // Grupos

    /// Crea un grupo con `creator` como dueño.
    pub fn create_group(&self, creator: &PlayerId, name: &str, password: Option<&str>) -> Result<(), GroupError> {
        let name = name.trim();
        // '|' es el separador en los mensajes hacia Java.
        if !valid_text(name, MAX_GROUP_NAME) || name.contains('|') || password.is_some_and(|p| !p.is_empty() && !valid_text(p, MAX_PASSWORD)) {
            return Err(GroupError::InvalidName);
        }
        self.with_state(|st, out| {
            if !st.players.contains_key(creator) {
                return Err(GroupError::UnknownPlayer);
            }
            if st.find_group(name).is_some() {
                return Err(GroupError::AlreadyExists);
            }
            let id = st.next_group;
            st.next_group += 1;
            st.groups.insert(
                id,
                Group {
                    name: name.to_owned(),
                    password: password.filter(|p| !p.is_empty()).map(str::to_owned),
                    owner: Some(*creator),
                    persistent: false,
                    saved_members: HashSet::new(),
                },
            );
            st.set_group(creator, Some(id), out);
            Ok(())
        })
    }

    pub fn join_group(&self, player: &PlayerId, name: &str, password: Option<&str>) -> Result<(), GroupError> {
        self.with_state(|st, out| {
            if !st.players.contains_key(player) {
                return Err(GroupError::UnknownPlayer);
            }
            let id = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            if let Some(expected) = &st.groups[&id].password {
                if password != Some(expected.as_str()) {
                    return Err(GroupError::WrongPassword);
                }
            }
            if st.players[player].group == Some(id) {
                return Ok(());
            }
            st.set_group(player, Some(id), out);
            st.push_event(EventKind::Joined, st.groups[&id].name.clone(), *player);
            Ok(())
        })
    }

    pub fn leave_group(&self, player: &PlayerId) -> bool {
        self.with_state(|st, out| {
            let Some(g) = st.players.get(player).and_then(|p| p.group) else { return false };
            let name = st.groups[&g].name.clone();
            st.set_group(player, None, out);
            if let Some(group) = st.groups.get_mut(&g) {
                if group.saved_members.remove(player) {
                    st.save();
                }
            }
            st.push_event(EventKind::Left, name, *player);
            st.after_member_left(g, player, out);
            true
        })
    }

    /// Expulsa a `target` de su grupo. Solo el dueño o un admin (`admin = true`).
    pub fn kick_from_group(&self, requester: &PlayerId, target: &PlayerId, admin: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.players.get(target).and_then(|p| p.group).ok_or(GroupError::NotInGroup)?;
            if !admin && st.groups[&g].owner != Some(*requester) {
                return Err(GroupError::NotOwner);
            }
            let name = st.groups[&g].name.clone();
            st.set_group(target, None, out);
            st.groups.get_mut(&g).unwrap().saved_members.remove(target);
            st.save();
            st.push_event(EventKind::Kicked, name.clone(), *target);
            st.after_member_left(g, target, out);
            Ok(name)
        })
    }

    /// Hace permanente (o temporal) el grupo de `requester`. Solo el dueño o un admin.
    pub fn set_group_persistent(&self, requester: &PlayerId, persistent: bool, admin: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.players.get(requester).and_then(|p| p.group).ok_or(GroupError::NotInGroup)?;
            if !admin && st.groups[&g].owner != Some(*requester) {
                return Err(GroupError::NotOwner);
            }
            Ok(st.apply_persistent(g, persistent, out))
        })
    }

    /// Borra un grupo, también si es permanente. Solo el dueño o un admin.
    pub fn delete_group(&self, requester: &PlayerId, name: &str, admin: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            if !admin && st.groups[&g].owner != Some(*requester) {
                return Err(GroupError::NotOwner);
            }
            let real_name = st.groups[&g].name.clone();
            for m in st.members_of(g) {
                st.set_group(&m, None, out);
            }
            st.groups.remove(&g);
            st.save();
            st.push_event(EventKind::Deleted, real_name.clone(), *requester);
            Ok(real_name)
        })
    }

    /// Mete a un jugador en un grupo sin pedir contraseña (plugins).
    pub fn force_join_group(&self, player: &PlayerId, name: &str) -> Result<(), GroupError> {
        self.with_state(|st, out| {
            if !st.players.contains_key(player) {
                return Err(GroupError::UnknownPlayer);
            }
            let id = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            if st.players[player].group == Some(id) {
                return Ok(());
            }
            st.set_group(player, Some(id), out);
            st.push_event(EventKind::Joined, st.groups[&id].name.clone(), *player);
            Ok(())
        })
    }

    /// Hace permanente (o temporal) un grupo por su nombre, sin mirar quién lo pide
    /// (plugins). Un grupo temporal sin nadie dentro se borra.
    pub fn set_group_persistent_by_name(&self, name: &str, persistent: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            let real = st.apply_persistent(g, persistent, out);
            st.cleanup_group(g);
            Ok(real)
        })
    }

    /// (dueño, contraseña sí/no, permanente) de un grupo.
    pub fn group_info(&self, name: &str) -> Option<(Option<PlayerId>, bool, bool)> {
        let st = self.state();
        let g = st.find_group(name.trim())?;
        let group = &st.groups[&g];
        Some((group.owner, group.password.is_some(), group.persistent))
    }

    pub fn group_of(&self, player: &PlayerId) -> Option<String> {
        let st = self.state();
        let g = st.players.get(player)?.group?;
        st.groups.get(&g).map(|g| g.name.clone())
    }

    pub fn group_owner(&self, name: &str) -> Option<PlayerId> {
        let st = self.state();
        st.find_group(name).and_then(|g| st.groups[&g].owner)
    }

    /// (nombre, miembros conectados, tiene contraseña, permanente), ordenado por nombre.
    pub fn list_groups(&self) -> Vec<(String, usize, bool, bool)> {
        let st = self.state();
        let mut out: Vec<_> = st
            .groups
            .iter()
            .map(|(id, g)| {
                let members = st.players.values().filter(|p| p.group == Some(*id)).count();
                (g.name.clone(), members, g.password.is_some(), g.persistent)
            })
            .collect();
        out.sort_by_key(|g| g.0.to_lowercase());
        out
    }

    pub fn group_members(&self, name: &str) -> Vec<PlayerId> {
        let st = self.state();
        st.find_group(name).map(|g| st.members_of(g)).unwrap_or_default()
    }

    /// Eventos pendientes, en orden.
    pub fn poll_events(&self) -> Vec<ServerEvent> {
        self.state().events.drain(..).collect()
    }

    // Oclusión calculada en el servidor

    /// Pares (hablante, oyente) de proximidad activos ahora, para que Java calcule
    /// la oclusión con el mundo del servidor.
    /// Parejas (hablante, oyente) que se oyen por separado, de más cerca a más lejos
    /// y como mucho [`ACTIVE_PAIRS_MAX`].
    pub fn active_pairs(&self) -> Vec<(PlayerId, PlayerId)> {
        let st = self.state();
        let now = Instant::now();
        let mut pairs: Vec<(f64, PlayerId, PlayerId)> = Vec::new();
        for (sid, s) in &st.players {
            if s.group.is_some() || !s.last_audio.is_some_and(|t| now.duration_since(t) < ACTIVE_SPEAKER) {
                continue;
            }
            if !st.role_rules.can_be_heard(s.role) {
                continue;
            }
            let r2 = (s.talk_range as f64) * (s.talk_range as f64);
            for (lid, l) in &st.players {
                if lid == sid
                    || l.dimension != s.dimension
                    || matches!(l.transport, Transport::Pending)
                    || !st.role_rules.can_hear(s.role, l.role)
                {
                    continue;
                }
                // Lo que le llega en el murmullo no necesita paredes.
                if !s.priority
                    && st.crowd_plans.get(&Cell::of(l.pos, l.dimension)).is_some_and(|near| !near.contains(sid))
                {
                    continue;
                }
                let d = [s.pos[0] - l.pos[0], s.pos[1] - l.pos[1], s.pos[2] - l.pos[2]];
                let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                if d2 <= r2 {
                    pairs.push((d2, *sid, *lid));
                }
            }
        }
        if pairs.len() > ACTIVE_PAIRS_MAX {
            pairs.select_nth_unstable_by(ACTIVE_PAIRS_MAX, |a, b| a.0.total_cmp(&b.0));
            pairs.truncate(ACTIVE_PAIRS_MAX);
        }
        pairs.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        pairs.into_iter().map(|(_, s, l)| (s, l)).collect()
    }

    pub fn set_occlusion(&self, speaker: &PlayerId, listener: &PlayerId, amount: f32) {
        self.set_occlusions(&[(*speaker, *listener, amount)]);
    }

    /// Varias de golpe (un solo lock).
    pub fn set_occlusions(&self, values: &[(PlayerId, PlayerId, f32)]) {
        let now = Instant::now();
        let mut st = self.state();
        for (speaker, listener, amount) in values {
            let q = (amount.clamp(0.0, 1.0) * 254.0).round() as u8;
            st.occlusion.insert((*speaker, *listener), (q, now));
        }
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
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.crowd_thread.take() {
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
    fn find_group(&self, name: &str) -> Option<u32> {
        self.groups.iter().find(|(_, g)| g.name.eq_ignore_ascii_case(name)).map(|(id, _)| *id)
    }

    fn members_of(&self, g: u32) -> Vec<PlayerId> {
        self.players.iter().filter(|(_, p)| p.group == Some(g)).map(|(id, _)| *id).collect()
    }

    fn push_event(&mut self, kind: EventKind, detail: String, player: PlayerId) {
        if self.events.len() >= EVENTS_MAX {
            self.events.pop_front();
        }
        self.events.push_back(ServerEvent { kind, player, detail });
    }

    /// Hace permanente o temporal un grupo, lo guarda y avisa a sus miembros.
    fn apply_persistent(&mut self, g: u32, persistent: bool, out: &mut Outgoing) -> String {
        let members: Vec<PlayerId> = self.members_of(g);
        let group = self.groups.get_mut(&g).unwrap();
        group.persistent = persistent;
        group.saved_members = if persistent { members.iter().copied().collect() } else { HashSet::new() };
        let name = group.name.clone();
        self.save();
        for m in members {
            self.send_group_state(&m, out);
        }
        name
    }

    /// Eventos de un jugador que deja el relé (sale, o se le vuelve a registrar).
    fn note_gone(&mut self, id: &PlayerId, p: &Player) {
        if p.speaking {
            self.push_event(EventKind::StoppedSpeaking, String::new(), *id);
        }
        if !matches!(p.transport, Transport::Pending) {
            self.push_event(EventKind::Disconnected, String::new(), *id);
        }
    }

    /// Cambia el grupo de un jugador y se lo notifica.
    fn set_group(&mut self, player: &PlayerId, group: Option<u32>, out: &mut Outgoing) {
        let old = match self.players.get_mut(player) {
            Some(p) => std::mem::replace(&mut p.group, group),
            None => return,
        };
        if let Some(g) = group {
            let grp = self.groups.get_mut(&g).unwrap();
            if grp.persistent && grp.saved_members.insert(*player) {
                self.save();
            }
        }
        self.send_group_state(player, out);
        if let Some(old) = old {
            if Some(old) != group {
                self.cleanup_group(old);
            }
        }
    }

    fn send_group_state(&mut self, player: &PlayerId, out: &mut Outgoing) {
        let state = match self.players.get(player).and_then(|p| p.group).and_then(|g| self.groups.get(&g)) {
            Some(g) => GroupState { name: g.name.clone(), owner: g.owner == Some(*player), persistent: g.persistent },
            None => GroupState::default(),
        };
        let body = state.encode();
        if let Some(p) = self.players.get_mut(player) {
            let pkt = p.seal(PacketType::GroupState, &body);
            // Contador local; las estadísticas aquí dan igual.
            let stats = ServerStats::default();
            deliver(&stats, p, pkt, out);
        }
    }

    /// Al salir alguien: si era dueño de un grupo temporal, pasa la propiedad; si el
    /// grupo se queda vacío, se borra.
    fn after_member_left(&mut self, g: u32, who: &PlayerId, out: &mut Outgoing) {
        let members = self.members_of(g);
        let Some(group) = self.groups.get_mut(&g) else { return };
        if !group.persistent && group.owner == Some(*who) {
            group.owner = members.first().copied();
            if let Some(new_owner) = group.owner {
                let name = group.name.clone();
                self.push_event(EventKind::NewOwner, name, new_owner);
                self.send_group_state(&new_owner, out);
            }
        }
        self.cleanup_group(g);
    }

    /// Borra los grupos temporales vacíos.
    fn cleanup_group(&mut self, id: u32) {
        let persistent = self.groups.get(&id).is_some_and(|g| g.persistent);
        if !persistent && !self.players.values().any(|p| p.group == Some(id)) {
            self.groups.remove(&id);
        }
    }

    // Fichero de datos

    fn serialize(&self) -> String {
        let mut s = String::from("# Ferrovoz: silencios y grupos permanentes. Edítalo solo con el servidor apagado.\n");
        let mut mutes: Vec<String> = self.admin_muted.iter().map(protocol::uuid_to_string).collect();
        mutes.sort();
        for m in mutes {
            s.push_str(&format!("mute\t{m}\n"));
        }
        let mut groups: Vec<&Group> = self.groups.values().filter(|g| g.persistent).collect();
        groups.sort_by_key(|g| g.name.to_lowercase());
        for g in groups {
            let owner = g.owner.as_ref().map(protocol::uuid_to_string).unwrap_or_else(|| "-".into());
            s.push_str(&format!("group\t{}\t{}\t{}\n", g.name, owner, g.password.as_deref().unwrap_or("")));
            let mut members: Vec<String> = g.saved_members.iter().map(protocol::uuid_to_string).collect();
            members.sort();
            for m in members {
                s.push_str(&format!("member\t{}\t{m}\n", g.name));
            }
        }
        s
    }

    fn load(&mut self, text: &str) {
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["mute", id] => {
                    if let Some(id) = protocol::uuid_from_str(id) {
                        self.admin_muted.insert(id);
                    }
                }
                ["group", name, owner, password] if valid_text(name, MAX_GROUP_NAME) && self.find_group(name).is_none() => {
                    let id = self.next_group;
                    self.next_group += 1;
                    self.groups.insert(
                        id,
                        Group {
                            name: (*name).to_owned(),
                            password: (!password.is_empty()).then(|| (*password).to_owned()),
                            owner: protocol::uuid_from_str(owner),
                            persistent: true,
                            saved_members: HashSet::new(),
                        },
                    );
                }
                ["member", group, id] => {
                    if let (Some(g), Some(id)) = (self.find_group(group), protocol::uuid_from_str(id)) {
                        self.groups.get_mut(&g).unwrap().saved_members.insert(id);
                    }
                }
                _ => {}
            }
        }
    }

    /// Escritura atómica (temporal + rename).
    fn save(&self) {
        let Some(path) = &self.data_file else { return };
        let tmp = path.with_extension("tmp");
        let result = std::fs::write(&tmp, self.serialize()).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = result {
            log::error!("could not save {}: {e}", path.display());
        }
    }
}

#[derive(Clone, Copy)]
enum Source {
    Udp(SocketAddr),
    Tunnel(PlayerId),
}

impl Shared {
    fn run(self: &Arc<Self>) {
        let mut buf = [0u8; MAX_PACKET];
        let stats = self.clone();
        let on_sent: Arc<dyn Fn(usize, u64) + Send + Sync> = Arc::new(move |sent, busy| {
            stats.stats.packets_out.fetch_add(sent as u64, Ordering::Relaxed);
            stats.stats.busy_ns.fetch_add(busy, Ordering::Relaxed);
        });
        // Los hilos auxiliares viven lo mismo que este bucle.
        let pool = match Pool::new(&self.socket, Pool::default_workers(), on_sent.clone()) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("voice relay: could not start helper threads ({e}), using one thread");
                Pool::new(&self.socket, 0, on_sent).expect("cloning the UDP socket")
            }
        };
        if pool.workers() > 0 {
            log::info!("voice relay: {} helper thread(s) for sending voice", pool.workers());
        }
        let mut jobs = Vec::new();
        let mut last_states = Instant::now();
        while self.running.load(Ordering::Relaxed) {
            // Estados de voz a todos, cada segundo.
            if last_states.elapsed() >= PEER_STATES_EVERY {
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
            if self.handle(&buf[..len], Source::Udp(from), &mut out, &mut jobs).is_err() {
                self.stats.rejected.fetch_add(1, Ordering::Relaxed);
            } else {
                self.send_udp(out);
            }
            if !jobs.is_empty() {
                pool.dispatch(std::mem::take(&mut jobs));
            }
            self.stats.busy_ns.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }

    /// Hilo de multitudes: cada 20 ms mezcla el murmullo de las zonas con multitud y
    /// cada 100 ms decide qué voces van por separado en cada zona.
    fn crowd_loop(self: &Arc<Self>) {
        let mut mixer = crowd::Mixer::new();
        let mut tick: u32 = 0;
        let mut next = Instant::now();
        while self.running.load(Ordering::Relaxed) {
            next += FRAME;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else if now - next > FRAME * 5 {
                next = now; // muy atrasado (máquina parada): no intentar recuperar
            }
            tick = tick.wrapping_add(1);
            if tick.is_multiple_of(CROWD_PLAN_EVERY) {
                self.plan_crowds();
                self.sweep_speaking();
                mixer.cleanup(Instant::now());
            }
            self.mix_crowds(&mut mixer, tick);
        }
    }

    /// Quien lleva un rato sin sonar deja de hablar.
    fn sweep_speaking(&self) {
        let mut st = self.state.lock().unwrap();
        let now = Instant::now();
        let quiet: Vec<PlayerId> = st
            .players
            .iter_mut()
            .filter(|(_, p)| p.speaking && !p.last_audio.is_some_and(|t| now.duration_since(t) < ACTIVE_SPEAKER))
            .map(|(id, p)| {
                p.speaking = false;
                *id
            })
            .collect();
        for id in quiet {
            st.push_event(EventKind::StoppedSpeaking, String::new(), id);
        }
    }

    /// Envía una trama de un canal a quien le toque. false si el canal no existe.
    fn route_channel(&self, id: &PlayerId, opus: &[u8], out: &mut Outgoing, jobs: &mut Vec<SealJob>) -> bool {
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;
        let Some(channel) = st.channels.get_mut(id) else { return false };
        channel.seq = channel.seq.wrapping_add(1);
        let (seq, range, target) = (channel.seq, channel.range, channel.target.clone());
        let origin = match &target {
            Target::Point { pos, dim } => Some((*pos, *dim)),
            Target::Follow(p) => match st.players.get(p) {
                Some(p) => Some((p.pos, p.dimension)),
                None => return true, // no está: se pierde la trama, el canal sigue
            },
            Target::Listeners(_) | Target::Group(_) => None,
        };
        let group_id = match &target {
            Target::Group(name) => match st.find_group(name) {
                Some(g) => Some(g),
                None => return true,
            },
            _ => None,
        };
        let rules = st.role_rules;
        let recipients: Vec<PlayerId> = st
            .players
            .iter()
            .filter(|(pid, l)| {
                if matches!(l.transport, Transport::Pending) {
                    return false;
                }
                match (&target, origin) {
                    (_, Some((pos, dim))) => {
                        if l.dimension != dim || !rules.can_hear(Role::Alive, l.role) {
                            return false;
                        }
                        let d = [pos[0] - l.pos[0], pos[1] - l.pos[1], pos[2] - l.pos[2]];
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= (range as f64) * (range as f64)
                    }
                    (Target::Listeners(set), None) => set.contains(*pid),
                    (Target::Group(_), None) => l.group == group_id,
                    _ => false,
                }
            })
            .map(|(pid, _)| *pid)
            .collect();
        let template = AudioDown {
            sender: *id,
            seq,
            rel: [0.0; 3],
            range: if origin.is_some() { range } else { 0.0 },
            group: origin.is_none(),
            underwater: false,
            env: Environment::default(),
            occlusion: OCCLUSION_UNKNOWN,
            opus: opus.to_vec(),
        }
        .encode();
        for pid in recipients {
            let listener = st.players.get_mut(&pid).unwrap();
            let rel = match origin {
                Some((pos, _)) => [
                    (pos[0] - listener.pos[0]) as f32,
                    (pos[1] - listener.pos[1]) as f32,
                    (pos[2] - listener.pos[2]) as f32,
                ],
                None => [0.0; 3],
            };
            let body = AudioDown::for_listener(&template, rel, OCCLUSION_UNKNOWN);
            match listener.transport {
                Transport::Udp(addr) => {
                    listener.tx_counter += 1;
                    jobs.push(SealJob { addr, key: listener.key, counter: listener.tx_counter, body });
                }
                _ => {
                    let pkt = listener.seal(PacketType::AudioDown, &body);
                    deliver(&self.stats, listener, pkt, out);
                }
            }
        }
        true
    }

    /// Para cada zona con oyentes, si le llegan más de `max_voices` voces, cuáles
    /// van por separado.
    fn plan_crowds(&self) {
        let mut st = self.state.lock().unwrap();
        if st.max_voices == 0 {
            st.crowd_plans.clear();
            return;
        }
        let now = Instant::now();
        let rules = st.role_rules;
        // Los de prioridad no cuentan: van siempre por separado.
        let speakers: Vec<crowd::Speaker> = st
            .players
            .iter()
            .filter(|(id, p)| {
                p.group.is_none()
                    && !p.priority
                    && p.can_speak
                    && !st.admin_muted.contains(*id)
                    && rules.can_be_heard(p.role)
                    && p.last_audio.is_some_and(|t| now.duration_since(t) < ACTIVE_SPEAKER)
            })
            .map(|(id, p)| crowd::Speaker { id: *id, pos: p.pos, dim: p.dimension, range: p.talk_range })
            .collect();
        if speakers.len() <= st.max_voices {
            st.crowd_plans.clear();
            return;
        }
        let cells: HashSet<Cell> = st
            .players
            .values()
            .filter(|p| !matches!(p.transport, Transport::Pending))
            .map(|p| Cell::of(p.pos, p.dimension))
            .collect();
        st.crowd_plans = crowd::plan(cells, &speakers, st.max_voices);
    }

    /// Mezcla y envía el murmullo de cada zona con multitud.
    fn mix_crowds(&self, mixer: &mut crowd::Mixer, tick: u32) {
        // 1. Con el lock, solo copiar lo necesario.
        let now = Instant::now();
        let (frames, cells) = {
            let mut st = self.state.lock().unwrap();
            if st.crowd_plans.is_empty() && st.crowd_stash.is_empty() {
                return;
            }
            let frames = st.crowd_stash.take(now);
            let cells: Vec<(Cell, HashSet<PlayerId>)> =
                st.crowd_plans.iter().map(|(c, near)| (*c, near.clone())).collect();
            (frames, cells)
        };
        if frames.is_empty() || cells.is_empty() {
            return;
        }

        // 2. Sin el lock: decodificar cada voz una vez y mezclar cada zona.
        let voices = mixer.decode(frames, now);
        let mixes: HashMap<Cell, crowd::CellMix> = cells
            .into_iter()
            .filter_map(|(cell, near)| mixer.mix(cell, &near, &voices, now).map(|m| (cell, m)))
            .collect();
        if mixes.is_empty() {
            return;
        }

        // 3. Con el lock: cifrar para cada oyente de esas zonas y enviar.
        let mut out = Vec::new();
        {
            let mut st = self.state.lock().unwrap();
            let rules = st.role_rules;
            let range = st.ranges.normal;
            for p in st.players.values_mut() {
                if matches!(p.transport, Transport::Pending) || !rules.can_hear(Role::Alive, p.role) {
                    continue;
                }
                let Some(mix) = mixes.get(&Cell::of(p.pos, p.dimension)) else { continue };
                // Dirección hacia la multitud, a una distancia que el cliente oye a
                // volumen completo (la mezcla ya lleva la atenuación de cada voz).
                let mut rel = [mix.centroid[0] - p.pos[0], mix.centroid[1] - p.pos[1], mix.centroid[2] - p.pos[2]];
                let len = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
                if len > CROWD_REL_DISTANCE {
                    for r in rel.iter_mut() {
                        *r *= CROWD_REL_DISTANCE / len;
                    }
                }
                let down = AudioDown {
                    sender: CROWD_ID,
                    // El mismo número en todas las zonas: al cambiar de zona el
                    // cliente sigue el murmullo sin saltos.
                    seq: tick,
                    rel: [rel[0] as f32, rel[1] as f32, rel[2] as f32],
                    range,
                    group: false,
                    underwater: false,
                    env: Environment::default(),
                    occlusion: OCCLUSION_UNKNOWN,
                    opus: mix.opus.clone(),
                };
                let pkt = p.seal(PacketType::AudioDown, &down.encode());
                deliver(&self.stats, p, pkt, &mut out);
            }
        }
        self.send_udp(out);
    }

    fn broadcast_peer_states(&self, out: &mut Outgoing) {
        let mut st = self.state.lock().unwrap();
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
            if st.bad_ips.len() > 4096 {
                st.bad_ips.clear();
            }
            st.bad_ips
                .entry(addr.ip())
                .or_insert_with(|| TokenBucket::new(BAD_PACKETS_RATE, BAD_PACKETS_BURST))
                .take();
        }
    }

    fn handle(&self, data: &[u8], source: Source, out: &mut Outgoing, jobs: &mut Vec<SealJob>) -> Result<(), ()> {
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;

        if let Source::Udp(addr) = source {
            if st.bad_ips.get_mut(&addr.ip()).is_some_and(|b| b.is_empty()) {
                self.stats.rate_limited.fetch_add(1, Ordering::Relaxed);
                return Err(());
            }
        }

        let raw = match protocol::parse(data) {
            Ok(r) => r,
            Err(_) => {
                Self::note_bad_packet(st, source);
                return Err(());
            }
        };
        let Some(sender_id) = raw.sender else {
            Self::note_bad_packet(st, source);
            return Err(());
        };
        if let Source::Tunnel(owner) = source {
            // Por el túnel solo puede hablar el dueño de esa conexión.
            if owner != sender_id {
                return Err(());
            }
        }
        let (cave_echo, hush, underwater_effect) = (st.cave_echo, st.hush, st.underwater_effect);
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

        let body = match raw.open_once(&sender.key, &mut sender.rx_window) {
            Ok(b) => b,
            Err(protocol::ProtocolError::Replay) => {
                self.stats.replayed.fetch_add(1, Ordering::Relaxed);
                return Err(());
            }
            Err(_) => {
                Self::note_bad_packet(st, source);
                return Err(());
            }
        };

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
                let frame = AudioUp::decode(&body).map_err(|_| ())?;
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

                let recipients: Vec<PlayerId> = st
                    .players
                    .iter()
                    .filter(|(id, l)| {
                        if **id == sender_id || matches!(l.transport, Transport::Pending) {
                            return false;
                        }
                        if !rules.can_hear(src_role, l.role) {
                            return false;
                        }
                        match group {
                            Some(g) => l.group == Some(g),
                            None => {
                                if l.dimension != src_dim {
                                    return false;
                                }
                                let d = [src_pos[0] - l.pos[0], src_pos[1] - l.pos[1], src_pos[2] - l.pos[2]];
                                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= (range as f64) * (range as f64)
                            }
                        }
                    })
                    .map(|(id, _)| *id)
                    .collect();

                // Multitud: a las zonas donde esta voz no va por separado le llega
                // en el murmullo.
                let mut recipients = recipients;
                if group.is_none() && !src_priority && !st.crowd_plans.is_empty() {
                    let before = recipients.len();
                    let (players, plans) = (&st.players, &st.crowd_plans);
                    recipients.retain(|id| {
                        let l = &players[id];
                        plans.get(&Cell::of(l.pos, l.dimension)).is_none_or(|near| near.contains(&sender_id))
                    });
                    if recipients.len() < before && src_role == Role::Alive {
                        let speaker = crowd::Speaker { id: sender_id, pos: src_pos, dim: src_dim, range };
                        st.crowd_stash.push(speaker, frame.opus.clone(), Instant::now());
                    }
                }

                let now = Instant::now();
                // La trama se monta una vez; para cada oyente solo cambian su
                // posición relativa y la oclusión.
                let template = AudioDown {
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
                .encode();
                for id in recipients {
                    let occlusion = match st.occlusion.get(&(sender_id, id)) {
                        Some((v, t)) if now.duration_since(*t) < OCCLUSION_TTL => *v,
                        _ => OCCLUSION_UNKNOWN,
                    };
                    let listener = st.players.get_mut(&id).unwrap();
                    let rel = [src_pos[0] - listener.pos[0], src_pos[1] - listener.pos[1], src_pos[2] - listener.pos[2]];
                    let body = AudioDown::for_listener(&template, [rel[0] as f32, rel[1] as f32, rel[2] as f32], occlusion);
                    match listener.transport {
                        // UDP: el contador se reserva aquí y se cifra fuera del lock.
                        Transport::Udp(addr) => {
                            listener.tx_counter += 1;
                            jobs.push(SealJob { addr, key: listener.key, counter: listener.tx_counter, body });
                        }
                        _ => {
                            let pkt = listener.seal(PacketType::AudioDown, &body);
                            deliver(&self.stats, listener, pkt, out);
                        }
                    }
                }
            }
            PacketType::VoiceState => {
                sender.self_flags = body.first().copied().unwrap_or(0) & (PeerStates::MUTED | PeerStates::DEAFENED);
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
            | PacketType::PeerStates => return Err(()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> VoiceServer {
        VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap()
    }

    fn kinds(s: &VoiceServer) -> Vec<(EventKind, String)> {
        s.poll_events().into_iter().map(|e| (e.kind, e.detail)).collect()
    }

    #[test]
    fn groups_create_join_leave_and_auto_cleanup() {
        let s = server();
        let (a, b) = ([1; 16], [2; 16]);
        s.add_player(a);
        s.add_player(b);
        assert_eq!(s.create_group(&a, "Equipo Rojo", Some("1234")), Ok(()));
        assert_eq!(s.create_group(&b, "equipo rojo", None), Err(GroupError::AlreadyExists));
        assert_eq!(s.join_group(&b, "EQUIPO ROJO", None), Err(GroupError::WrongPassword));
        assert_eq!(s.join_group(&b, "equipo rojo", Some("1234")), Ok(()));
        assert_eq!(s.group_of(&b).as_deref(), Some("Equipo Rojo"));
        assert_eq!(s.group_owner("equipo rojo"), Some(a));
        assert_eq!(s.list_groups(), vec![("Equipo Rojo".to_string(), 2, true, false)]);
        assert!(s.leave_group(&a));
        assert_eq!(s.group_owner("equipo rojo"), Some(b), "el dueño se va: la corona pasa a otro miembro");
        assert!(s.leave_group(&b));
        assert!(s.list_groups().is_empty(), "un grupo temporal vacío desaparece");
        assert_eq!(
            kinds(&s),
            vec![
                (EventKind::Joined, "Equipo Rojo".into()),
                (EventKind::Left, "Equipo Rojo".into()),
                (EventKind::NewOwner, "Equipo Rojo".into()),
                (EventKind::Left, "Equipo Rojo".into()),
            ]
        );
        assert_eq!(s.create_group(&a, "   ", None), Err(GroupError::InvalidName));
        assert_eq!(s.create_group(&a, "con\ttab", None), Err(GroupError::InvalidName));
        assert_eq!(s.create_group(&a, "a|b", None), Err(GroupError::InvalidName));
        assert_eq!(s.create_group(&a, "ok", Some("pa|ss")), Ok(()), "la contraseña sí puede llevar '|'");
    }

    #[test]
    fn kick_requires_owner_or_admin() {
        let s = server();
        let (owner, member, other) = ([1; 16], [2; 16], [3; 16]);
        for p in [owner, member, other] {
            s.add_player(p);
        }
        s.create_group(&owner, "club", None).unwrap();
        s.join_group(&member, "club", None).unwrap();
        s.join_group(&other, "club", None).unwrap();
        assert_eq!(s.kick_from_group(&other, &member, false), Err(GroupError::NotOwner));
        assert_eq!(s.kick_from_group(&owner, &member, false), Ok("club".into()));
        assert_eq!(s.group_of(&member), None);
        assert_eq!(s.kick_from_group(&member, &other, true), Ok("club".into()), "un admin puede");
        assert_eq!(s.kick_from_group(&owner, &other, false), Err(GroupError::NotInGroup));
    }

    #[test]
    fn persistent_groups_and_mutes_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("ferrovoz-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.txt");
        let _ = std::fs::remove_file(&file);
        let (a, b, troll) = ([1; 16], [2; 16], [9; 16]);
        {
            let s = server();
            s.set_data_file(&file).unwrap();
            s.add_player(a);
            s.add_player(b);
            s.create_group(&a, "Piratas", Some("loro")).unwrap();
            s.join_group(&b, "piratas", Some("loro")).unwrap();
            assert_eq!(s.set_group_persistent(&b, true, false), Err(GroupError::NotOwner));
            assert_eq!(s.set_group_persistent(&a, true, false), Ok("Piratas".into()));
            s.set_muted(&troll, true);
            s.remove_player(&a);
            s.remove_player(&b);
            assert_eq!(s.list_groups(), vec![("Piratas".to_string(), 0, true, true)], "permanente aunque se vacíe");
        }
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("group\tPiratas\t01010101-0101-0101-0101-010101010101\tloro"), "{text}");
        // Reinicio: servidor nuevo con el mismo fichero.
        let s = server();
        s.set_data_file(&file).unwrap();
        assert!(s.is_muted(&troll), "el silencio sobrevive al reinicio");
        assert_eq!(s.list_groups().len(), 1);
        s.add_player(b);
        assert_eq!(s.group_of(&b).as_deref(), Some("Piratas"), "al volver entra solo en su grupo permanente");
        assert_eq!(kinds(&s), vec![(EventKind::Rejoined, "Piratas".into())]);
        assert_eq!(s.group_owner("piratas"), Some(a), "el dueño se mantiene aunque no esté conectado");
        assert_eq!(s.delete_group(&b, "piratas", false), Err(GroupError::NotOwner));
        assert_eq!(s.delete_group(&b, "piratas", true), Ok("Piratas".into()));
        assert!(!std::fs::read_to_string(&file).unwrap().contains("Piratas"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loss_meter_measures_gaps() {
        let mut m = LossMeter::default();
        for seq in 0..400u32 {
            if seq % 4 != 3 {
                m.record(seq);
            }
        }
        assert!((20..=30).contains(&m.percent), "≈25 %: {}", m.percent);
        let mut ok = LossMeter::default();
        for seq in 0..400u32 {
            ok.record(seq);
        }
        assert_eq!(ok.percent, 0);
    }

    #[test]
    fn token_bucket_limits_bursts() {
        let mut b = TokenBucket::new(10.0, 5.0);
        let allowed = (0..50).filter(|_| b.take()).count();
        assert_eq!(allowed, 5, "solo la ráfaga inicial");
        std::thread::sleep(Duration::from_millis(220));
        assert!(b.take() && b.take(), "se recarga con el tiempo");
    }

    #[test]
    fn tunnel_rejects_packets_from_another_player() {
        let s = server();
        let (a, b) = ([1; 16], [2; 16]);
        let key_b = s.add_player(b);
        s.add_player(a);
        let hello_b = protocol::seal(&key_b, PacketType::Hello, Some(&b), &[], 1);
        s.handle_tunnel(&a, &hello_b);
        assert_eq!(s.connected_count(), 0);
        s.handle_tunnel(&b, &hello_b);
        assert_eq!(s.tunnel_count(), 1);
        let pending = s.poll_tunnel(&b);
        assert_eq!(pending.len(), 2, "recibe su Pong y su estado de grupo por el túnel");
        // El mismo Hello otra vez: lo rechaza el anti-replay.
        s.handle_tunnel(&b, &hello_b);
        assert_eq!(s.stats().replayed.load(Ordering::Relaxed), 1);
    }
}
