//! Reenviador de voz para proxies (Velocity, BungeeCord).
//!
//! Detrás de un proxy cada servidor tiene su relé, normalmente en una red privada.
//! El reenviador corre en el proxy, escucha en un solo puerto público y manda la voz
//! de cada jugador al relé de su servidor:
//!
//! ```text
//!  cliente --UDP--> proxy:24454 (reenviador) --> servidor A:24455 (relé)
//!                                            \-> servidor B:24456 (relé)
//! ```
//!
//! - Cada jugador usa su propio socket hacia el relé, así se sabe de quién es cada
//!   respuesta.
//! - El reenviador conoce la clave de cada jugador (va en el mensaje de setup que
//!   intercepta el plugin) y solo deja pasar paquetes auténticos y nuevos, con la
//!   misma ventana anti-replay que el relé. La dirección de un jugador solo cambia
//!   con uno de esos.
//! - Un hilo para todos (`mio`).
//!
//! Grupos de red: los miembros se oyen estén en el servidor que estén. Su voz no
//! sube al servidor; la reparte el reenviador a los demás miembros, cifrada con la
//! clave de cada uno y con dirección `Relay` en el nonce para no chocar con el relé.

use crate::sync::lock;
use crate::protocol::{
    self, AudioDown, AudioUp, NetGroupState, PacketType, PlayerId, PlayerKey, ReplayWindow, MAX_PACKET,
    OCCLUSION_UNKNOWN,
};
use crate::server::{TokenBucket, AUDIO_BURST, AUDIO_RATE};
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token, Waker};
use std::collections::{HashMap, HashSet};
use std::io::{self, ErrorKind};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const FRONT: Token = Token(0);
const WAKE: Token = Token(1);

#[derive(Default)]
pub struct ForwarderStats {
    /// Paquetes de clientes reenviados a los relés.
    pub packets_up: AtomicU64,
    /// Paquetes de relés reenviados a clientes.
    pub packets_down: AtomicU64,
    /// Basura, jugadores sin ruta o paquetes que no se pudieron descifrar.
    pub rejected: AtomicU64,
    /// Replays o intentos de cambiar la dirección de otro jugador.
    pub replayed: AtomicU64,
    /// Voz de grupos de red entregada por el proxy (un paquete por oyente).
    pub packets_relayed: AtomicU64,
}

enum Command {
    Route { id: PlayerId, key: PlayerKey, backend: SocketAddr },
    Remove { id: PlayerId },
    NetGroup { group: u32, name: String, owner: Option<PlayerId>, members: Vec<(PlayerId, String)> },
}

struct NetGroup {
    name: String,
    owner: Option<PlayerId>,
    members: Vec<(PlayerId, String)>,
}

struct Shared {
    commands: Mutex<Vec<Command>>,
    running: AtomicBool,
    routes: AtomicUsize,
    stats: ForwarderStats,
}

pub struct VoiceForwarder {
    shared: Arc<Shared>,
    waker: Arc<Waker>,
    local: SocketAddr,
    thread: Option<JoinHandle<()>>,
}

impl VoiceForwarder {
    pub fn start(bind: SocketAddr) -> io::Result<Self> {
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), WAKE)?);
        let mut front = UdpSocket::bind(bind)?;
        let local = front.local_addr()?;
        poll.registry().register(&mut front, FRONT, Interest::READABLE)?;
        let shared = Arc::new(Shared {
            commands: Mutex::new(Vec::new()),
            running: AtomicBool::new(true),
            routes: AtomicUsize::new(0),
            stats: ForwarderStats::default(),
        });
        let worker = Worker {
            shared: shared.clone(),
            poll,
            front,
            routes: HashMap::new(),
            by_token: HashMap::new(),
            next_token: 2,
            groups: HashMap::new(),
            member_of: HashMap::new(),
            relay_counter: initial_relay_counter(),
            relay_buckets: HashMap::new(),
        };
        let thread = std::thread::Builder::new().name("ferrovoz-forwarder".into()).spawn(move || worker.run())?;
        log::info!("voice forwarder listening on {local}");
        Ok(Self { shared, waker, local, thread: Some(thread) })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// El jugador `id` (con esta clave) está en el servidor con relé `backend`.
    /// Si ya tenía ruta, se sustituye.
    pub fn route(&self, id: PlayerId, key: PlayerKey, backend: SocketAddr) {
        self.command(Command::Route { id, key, backend });
    }

    pub fn remove(&self, id: &PlayerId) {
        self.command(Command::Remove { id: *id });
    }

    /// Miembros del grupo de red `group` (UUID de voz y nombre). Sin miembros se
    /// borra. Se avisa a todos los afectados.
    pub fn set_net_group(&self, group: u32, name: String, owner: Option<PlayerId>, members: Vec<(PlayerId, String)>) {
        self.command(Command::NetGroup { group, name, owner, members });
    }

    pub fn routes(&self) -> usize {
        self.shared.routes.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> &ForwarderStats {
        &self.shared.stats
    }

    fn command(&self, c: Command) {
        lock(&self.shared.commands).push(c);
        let _ = self.waker.wake();
    }

    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::SeqCst);
        let _ = self.waker.wake();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for VoiceForwarder {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Route {
    key: PlayerKey,
    backend: SocketAddr,
    upstream: UdpSocket,
    token: Token,
    client: Option<SocketAddr>,
    window: ReplayWindow,
}

struct Worker {
    shared: Arc<Shared>,
    poll: Poll,
    front: UdpSocket,
    routes: HashMap<PlayerId, Route>,
    by_token: HashMap<Token, PlayerId>,
    next_token: usize,
    groups: HashMap<u32, NetGroup>,
    member_of: HashMap<PlayerId, u32>,
    /// Contador de lo que envía el proxy (dirección `Relay`). Basta uno para todos
    /// mientras suba siempre.
    relay_counter: u64,
    /// Límite de voz por jugador, para que nadie use el proxy para multiplicar tráfico.
    relay_buckets: HashMap<PlayerId, TokenBucket>,
}

/// Microsegundos desde 1970, para que los contadores sigan subiendo si se reinicia.
fn initial_relay_counter() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(1)
}

impl Worker {
    fn run(mut self) {
        let mut events = Events::with_capacity(256);
        let mut buf = [0u8; MAX_PACKET];
        while self.shared.running.load(Ordering::Relaxed) {
            if let Err(e) = self.poll.poll(&mut events, Some(Duration::from_millis(250))) {
                if e.kind() != ErrorKind::Interrupted {
                    log::warn!("voice forwarder: {e}");
                }
                continue;
            }
            for event in events.iter() {
                match event.token() {
                    WAKE => self.apply_commands(),
                    FRONT => self.drain_front(&mut buf),
                    token => self.drain_upstream(token, &mut buf),
                }
            }
        }
        self.routes.clear();
        self.shared.routes.store(0, Ordering::Relaxed);
    }

    fn apply_commands(&mut self) {
        let commands = std::mem::take(&mut *lock(&self.shared.commands));
        for c in commands {
            match c {
                Command::Route { id, key, backend } => {
                    self.remove(&id);
                    // Misma familia que el relé (IPv4 o IPv6), puerto libre.
                    let local: SocketAddr = match backend.ip() {
                        IpAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
                        IpAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
                    };
                    let token = Token(self.next_token);
                    self.next_token += 1;
                    match UdpSocket::bind(local).and_then(|mut s| {
                        self.poll.registry().register(&mut s, token, Interest::READABLE)?;
                        Ok(s)
                    }) {
                        Ok(upstream) => {
                            self.by_token.insert(token, id);
                            self.routes.insert(id, Route { key, backend, upstream, token, client: None, window: ReplayWindow::default() });
                        }
                        Err(e) => log::warn!("voice forwarder: could not open a socket for {}: {e}", protocol::uuid_to_string(&id)),
                    }
                }
                Command::Remove { id } => self.remove(&id),
                Command::NetGroup { group, name, owner, members } => self.set_group(group, name, owner, members),
            }
        }
        self.shared.routes.store(self.routes.len(), Ordering::Relaxed);
    }

    fn remove(&mut self, id: &PlayerId) {
        if let Some(mut r) = self.routes.remove(id) {
            let _ = self.poll.registry().deregister(&mut r.upstream);
            self.by_token.remove(&r.token);
        }
    }

    fn set_group(&mut self, group: u32, name: String, owner: Option<PlayerId>, members: Vec<(PlayerId, String)>) {
        let before: HashSet<PlayerId> = self
            .groups
            .get(&group)
            .map(|g| g.members.iter().map(|(id, _)| *id).collect())
            .unwrap_or_default();
        let now: HashSet<PlayerId> = members.iter().map(|(id, _)| *id).collect();
        for gone in before.difference(&now) {
            if self.member_of.get(gone) == Some(&group) {
                self.member_of.remove(gone);
                self.send_group_state(gone); // sin grupo de red
            }
        }
        if members.is_empty() {
            self.groups.remove(&group);
            return;
        }
        for id in &now {
            // Como mucho un grupo de red por jugador.
            if let Some(old) = self.member_of.insert(*id, group) {
                if old != group {
                    if let Some(g) = self.groups.get_mut(&old) {
                        g.members.retain(|(m, _)| m != id);
                    }
                }
            }
        }
        self.groups.insert(group, NetGroup { name, owner, members });
        for id in &now {
            self.send_group_state(id);
        }
    }

    /// Envía a `id` su grupo de red (o ninguno).
    fn send_group_state(&mut self, id: &PlayerId) {
        let state = match self.member_of.get(id).and_then(|g| self.groups.get(g)) {
            Some(g) => NetGroupState { name: g.name.clone(), owner: g.owner == Some(*id), members: g.members.clone() },
            None => NetGroupState::default(),
        };
        let Some(route) = self.routes.get(id) else { return };
        let Some(client) = route.client else { return };
        self.relay_counter += 1;
        let pkt = protocol::seal(&route.key, PacketType::RelayGroup, None, &state.encode(), self.relay_counter);
        let _ = self.front.send_to(&pkt, client);
    }

    /// Voz de un miembro de un grupo de red: a los demás miembros.
    fn relay_group_voice(&mut self, from: PlayerId, group: u32, body: &[u8]) {
        let bucket = self.relay_buckets.entry(from).or_insert_with(|| TokenBucket::new(AUDIO_RATE, AUDIO_BURST));
        if !bucket.take() {
            return;
        }
        let Ok(frame) = AudioUp::decode(body) else {
            self.shared.stats.rejected.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let Some(g) = self.groups.get(&group) else { return };
        let down = AudioDown {
            sender: from,
            seq: frame.seq,
            rel: [0.0; 3],
            range: 0.0,
            group: true,
            underwater: false,
            env: frame.env,
            occlusion: OCCLUSION_UNKNOWN,
            opus: frame.opus,
        }
        .encode();
        for (id, _) in &g.members {
            if *id == from {
                continue;
            }
            let Some(route) = self.routes.get(id) else { continue };
            let Some(client) = route.client else { continue };
            self.relay_counter += 1;
            let pkt = protocol::seal(&route.key, PacketType::RelayAudio, None, &down, self.relay_counter);
            if self.front.send_to(&pkt, client).is_ok() {
                self.shared.stats.packets_relayed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Clientes -> relé de su servidor.
    fn drain_front(&mut self, buf: &mut [u8]) {
        loop {
            let (len, src) = match self.front.recv_from(buf) {
                Ok(v) => v,
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(_) => continue, // p. ej. ICMP de puerto inalcanzable en Windows
            };
            let data = &buf[..len];
            let stats = &self.shared.stats;
            let Ok(raw) = protocol::parse(data) else {
                stats.rejected.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let Some(id) = raw.sender else {
                stats.rejected.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let Some(route) = self.routes.get_mut(&id) else {
                stats.rejected.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            // Solo paquetes auténticos y nuevos; el relé lo vuelve a comprobar.
            let body = match raw.open_once(&route.key, &mut route.window) {
                Ok(body) => body,
                Err(protocol::ProtocolError::Replay) => {
                    stats.replayed.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                Err(_) => {
                    stats.rejected.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            // Dirección actual del jugador (puede cambiar por NAT).
            route.client = Some(src);
            let group = self.member_of.get(&id).copied();
            // En un grupo de red la voz la reparte el proxy y no sube al servidor.
            if let (PacketType::AudioUp, Some(g)) = (raw.kind, group) {
                self.relay_group_voice(id, g, &body);
                continue;
            }
            if route.upstream.send_to(data, route.backend).is_ok() {
                stats.packets_up.fetch_add(1, Ordering::Relaxed);
            }
            // Cliente nuevo (cambio de servidor, voz reactivada...): reenviar su grupo.
            if raw.kind == PacketType::Hello && group.is_some() {
                self.send_group_state(&id);
            }
        }
    }

    /// Relé -> jugador de ese socket.
    fn drain_upstream(&mut self, token: Token, buf: &mut [u8]) {
        let Some(id) = self.by_token.get(&token).copied() else { return };
        let Some(route) = self.routes.get_mut(&id) else { return };
        loop {
            let (len, from) = match route.upstream.recv_from(buf) {
                Ok(v) => v,
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(_) => continue,
            };
            // Solo lo que venga del relé de su servidor.
            if from != route.backend {
                self.shared.stats.rejected.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if let Some(client) = route.client {
                if self.front.send_to(&buf[..len], client).is_ok() {
                    self.shared.stats.packets_down.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}
