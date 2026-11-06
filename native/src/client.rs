//! Cliente de voz.
//!
//! Subida: micro -> AEC -> RNNoise -> AGC -> Opus -> cifrado -> UDP o túnel.
//! Bajada: descifrado -> jitter buffer -> Opus -> volumen, oclusión, 3D, reverb -> mezcla.
//!
//! No depende de los dispositivos de audio: `push_capture` recibe mono a 48 kHz y
//! `render` devuelve estéreo intercalado a 48 kHz. `audio` lo conecta con cpal y los
//! tests lo usan sin hardware.
//!
//! Si el servidor no contesta por UDP en 3 s se pasa al túnel: los paquetes salen por
//! `poll_tunnel`, entran por `handle_tunnel` y Java los lleva por la conexión del juego.
//! `start_local` arranca sin red (prueba de micro en la pantalla de ajustes).

use crate::codec::{VoiceDecoder, VoiceEncoder};
use crate::denoise::Denoiser;
use crate::micproc::MicProcessor;
use crate::jitter::{JitterBuffer, Pop};
use crate::protocol::{
    self, AudioDown, AudioUp, Environment, GroupState, NetGroupState, PacketType, PeerStates, PlayerId, PlayerKey, PongInfo, ReplayWindow,
    TalkMode, FRAME_SAMPLES, MAX_PACKET, OCCLUSION_UNKNOWN, SAMPLE_RATE,
};
use crate::reverb::{EarlyReflections, Reflection, Reverb};
use crate::hrtf::{Binaural, Direction};
use crate::spatial::{self, SpatialParams};
use crate::underwater::{self, Underwater};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Tramas que se siguen enviando al dejar de detectar voz (300 ms), para no cortar
/// los finales de palabra.
const VAD_HANGOVER_FRAMES: u32 = 15;
/// Hablantes sin paquetes durante este tiempo se eliminan.
const SPEAKER_TIMEOUT: Duration = Duration::from_secs(30);
/// Margen para considerar que alguien está hablando.
const SPEAKING_WINDOW: Duration = Duration::from_millis(250);
/// Máximo de audio propio pendiente en la prueba de micro (~200 ms).
const MONITOR_MAX: usize = FRAME_SAMPLES * 10;
/// Sin respuesta por UDP en este tiempo se pasa al túnel.
const UDP_FALLBACK_AFTER: Duration = Duration::from_secs(3);
/// Cola máxima de subida por el túnel (se descartan los más viejos).
const TUNNEL_QUEUE_MAX: usize = 100;
/// Volumen de la voz de grupo.
const GROUP_GAIN: f32 = 0.85;
/// Cuánto de la voz propia entra en la reverb.
const SELF_ECHO_GAIN: f32 = 0.8;
/// La voz propia solo entra en la reverb con `room` >= 0.2 (RT60 ~0.9 s), del todo a ~1.5 s.
const SELF_ECHO_MIN_ROOM: f32 = 0.2;
/// Energía máxima de las reflexiones tempranas sumadas, relativa a la voz (-6 dB).
const MAX_EARLY_ENERGY: f32 = 0.25;
/// Atenuación por cada esquina del camino (difracción).
const CORNER_GAIN: f32 = 0.6;
/// Corte del paso bajo por esquina.
const CORNER_CUTOFF: f32 = 9_000.0;
/// Validez de un camino enviado por Java.
const PATH_TTL: Duration = Duration::from_millis(1_000);
/// Tramas grabadas pendientes de recoger (~10 s con 10 hablantes).
const RECORDED_MAX: usize = 5_000;
/// Validez de una posición de repetición.
const POSITION_TTL: Duration = Duration::from_millis(500);
/// Los estados llegan cada segundo; pasado esto se descarta la entrada.
const PEER_STATE_TTL: Duration = Duration::from_secs(3);
/// Cada cuánto se reenvía el estado propio aunque no cambie.
const VOICE_STATE_EVERY: Duration = Duration::from_secs(5);
/// Nivel de las reflexiones tempranas.
const EARLY_LEVEL: f32 = 0.7;
/// 3 s de ceros exactos: probablemente el sistema bloquea el micro.
const MIC_SILENT_SAMPLES: usize = SAMPLE_RATE as usize * 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TransmitMode {
    PushToTalk = 0,
    VoiceActivation = 1,
}

/// Transportes permitidos (lo decide el servidor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TransportPolicy {
    /// UDP, con túnel de reserva.
    Auto = 0,
    UdpOnly = 1,
    TunnelOnly = 2,
}

impl TransportPolicy {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::UdpOnly,
            2 => Self::TunnelOnly,
            _ => Self::Auto,
        }
    }
}

/// Estadísticas de red para la UI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetStats {
    pub rtt_ms: u32,
    pub loss_percent: u8,
    pub bitrate: u32,
    pub tunnel: bool,
}

pub(crate) struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub(crate) fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

/// Escala las reflexiones para que su energía total no pase de `MAX_EARLY_ENERGY`.
/// Sin esto, en salas pequeñas llegan todas a la vez y saturan.
fn limit_early_energy(mut reflections: Vec<Reflection>) -> Vec<Reflection> {
    let energy: f32 = reflections.iter().map(|r| r.gain * r.gain).sum();
    if energy > MAX_EARLY_ENERGY {
        let k = (MAX_EARLY_ENERGY / energy).sqrt();
        for r in &mut reflections {
            r.gain *= k;
        }
    }
    reflections
}

/// Contador inicial: microsegundos desde 1970. Así una reconexión con la misma clave
/// no reutiliza nonces ni la descarta la ventana anti-replay del servidor. Dentro del
/// proceso siempre crece, aunque el reloj vaya hacia atrás.
fn initial_counter() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    // Reserva 2^32 paquetes por sesión.
    let start = LAST.fetch_max(now, Ordering::Relaxed).max(now);
    LAST.store(start + (1 << 32), Ordering::Relaxed);
    start
}

/// Bitrate según la pérdida que reporta el servidor.
pub fn bitrate_for_loss(loss_percent: u8) -> u32 {
    match loss_percent {
        0..=2 => 32_000,
        3..=7 => 24_000,
        8..=14 => 18_000,
        _ => 14_000,
    }
}

struct CaptureState {
    encoder: VoiceEncoder,
    applied_bitrate: u32,
    denoiser: Denoiser,
    frame: Vec<f32>,
    seq: u32,
    hangover: u32,
    /// Muestras seguidas a cero exacto. Un micro real siempre tiene algo de ruido;
    /// ceros puros suelen ser un permiso denegado (macOS) o un mute por hardware.
    zero_run: usize,
}

struct Speaker {
    jitter: JitterBuffer,
    decoder: VoiceDecoder,
    rel: [f32; 3],
    range: f32,
    group: bool,
    last_packet: Instant,
    /// Oclusión suavizada (0 = libre, 1 = muy tapado).
    occlusion: f32,
    /// Oclusión calculada por el servidor, para hablantes fuera del alcance del cliente.
    server_occlusion: Option<f32>,
    /// Reverb del sitio del hablante.
    env: Environment,
    lowpass: f32,
    /// El hablante tiene la cabeza bajo el agua.
    underwater: bool,
    water: Underwater,
    /// HRTF.
    binaural: Binaural,
    /// Voz que llega rodeando un obstáculo: su propio 3D, filtro y ganancia.
    path_binaural: Binaural,
    path_lowpass: f32,
    path_gain: f32,
}

/// Camino libre por el que llega la voz cuando la línea recta está tapada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundPath {
    /// Dirección del hueco por el que entra (coordenadas del mundo, unitaria).
    pub dir: [f32; 3],
    /// Longitud en bloques.
    pub length: f32,
    /// Esquinas.
    pub corners: u32,
}

/// Acústica de la sala del oyente (la mide Java).
#[derive(Debug, Clone, Default)]
struct Acoustics {
    damp: f32,
    reflections: Vec<Reflection>,
}

struct Mixer {
    speakers: HashMap<PlayerId, Speaker>,
    /// Volumen por jugador (1.0 normal, 0.0 silenciado).
    player_volume: HashMap<PlayerId, f32>,
    /// Oclusión objetivo por jugador (Java).
    occlusion: HashMap<PlayerId, f32>,
    /// Caminos alternativos por jugador.
    paths: HashMap<PlayerId, (SoundPath, Instant)>,
    /// Repeticiones: posición de cada hablante relativa a la cámara.
    positions: HashMap<PlayerId, ([f32; 3], Instant)>,
    /// Estéreo intercalado a 48 kHz pendiente de entregar.
    ready: VecDeque<f32>,
    /// Voz propia para la prueba de micro.
    monitor: VecDeque<f32>,
    /// Voz propia para la reverb.
    self_echo: VecDeque<f32>,
    reverb: Reverb,
    /// Reverb del sitio del oyente.
    listener_env: (f32, f32),
    acoustics: Acoustics,
    early: EarlyReflections,
    params: SpatialParams,
}

/// Dispositivos elegidos. Cadena vacía = predeterminado.
#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct DeviceSelection {
    pub input: String,
    pub output: String,
}

struct NetState {
    policy: TransportPolicy,
    tunnel_active: bool,
    tunnel_out: VecDeque<Vec<u8>>,
    started: Instant,
    ping_sent: Option<Instant>,
    rx_window: ReplayWindow,
    /// Ventana anti-replay para los paquetes del proxy.
    relay_window: ReplayWindow,
}

pub(crate) struct Shared {
    socket: Option<UdpSocket>,
    networked: bool,
    id: PlayerId,
    key: PlayerKey,
    running: AtomicBool,
    connected: AtomicBool,
    ptt_down: AtomicBool,
    talk: AtomicU32,
    mode: AtomicU32,
    muted: AtomicBool,
    deafened: AtomicBool,
    talking: AtomicBool,
    monitor: AtomicBool,
    denoise: AtomicBool,
    /// Ajustes: reverb y efecto submarino.
    cave_echo: AtomicBool,
    underwater_fx: AtomicBool,
    /// El oyente tiene la cabeza bajo el agua.
    listener_underwater: AtomicBool,
    vad_threshold: AtomicF32,
    mic_gain: AtomicF32,
    volume: AtomicF32,
    yaw: AtomicF32,
    /// Pitch del oyente (grados, positivo = abajo).
    pitch: AtomicF32,
    /// HRTF en vez de paneo estéreo.
    hrtf: AtomicBool,
    mic_level: AtomicF32,
    mic_silent: AtomicBool,
    rtt_ms: AtomicU32,
    loss_percent: AtomicU32,
    target_bitrate: AtomicU32,
    tx_counter: AtomicU64,
    /// wet y room cuantizados, se envían con la voz.
    my_env: AtomicU32,
    group: Mutex<GroupState>,
    /// Grupo de red del proxy (nombre vacío = ninguno).
    net_group: Mutex<NetGroupState>,
    /// Grabación de repetición activa: se guardan las tramas propias y recibidas.
    recording: AtomicBool,
    recorded: Mutex<VecDeque<Vec<u8>>>,
    /// Estado de voz de los demás: flags e instante de llegada. `None` si el servidor
    /// no lo envía (versiones anteriores a 0.10).
    peers: Mutex<Option<HashMap<PlayerId, (u8, Instant)>>>,
    /// Volumen de la voz de grupo.
    group_volume: AtomicF32,
    devices: Mutex<DeviceSelection>,
    /// Cambia al cambiar de dispositivo; el hilo de audio reabre los streams.
    device_generation: AtomicU32,
    /// 0 = sin abrir, 1 = abiertos, 2 = error.
    pub(crate) audio_status: AtomicU32,
    net: Mutex<NetState>,
    capture: Mutex<CaptureState>,
    mixer: Mutex<Mixer>,
    /// AEC y AGC. Orden de locks: capture o mixer antes que este.
    mic_proc: Mutex<MicProcessor>,
}

pub struct VoiceClient {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl VoiceClient {
    /// Conecta por UDP (con túnel de reserva). `with_audio` abre los dispositivos
    /// (feature `audio`); los tests pasan `false`.
    pub fn start(server: SocketAddr, id: PlayerId, key: PlayerKey, with_audio: bool) -> io::Result<Self> {
        Self::start_with(server, id, key, with_audio, TransportPolicy::Auto)
    }

    pub fn start_with(
        server: SocketAddr,
        id: PlayerId,
        key: PlayerKey,
        with_audio: bool,
        policy: TransportPolicy,
    ) -> io::Result<Self> {
        let socket = if policy == TransportPolicy::TunnelOnly {
            None
        } else {
            let bind: SocketAddr = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse().unwrap();
            let socket = UdpSocket::bind(bind)?;
            socket.connect(server)?;
            socket.set_read_timeout(Some(Duration::from_millis(250)))?;
            Some(socket)
        };
        Self::build(socket, true, policy, id, key, with_audio)
    }

    /// Sin red: solo dispositivos, para la prueba de micro.
    pub fn start_local(with_audio: bool) -> io::Result<Self> {
        Self::build(None, false, TransportPolicy::UdpOnly, [0; 16], [0; 32], with_audio)
    }

    fn build(
        socket: Option<UdpSocket>,
        networked: bool,
        policy: TransportPolicy,
        id: PlayerId,
        key: PlayerKey,
        with_audio: bool,
    ) -> io::Result<Self> {
        let initial_bitrate = bitrate_for_loss(0);
        let encoder = VoiceEncoder::new(initial_bitrate as i32).map_err(io::Error::other)?;
        let has_socket = socket.is_some();
        let shared = Arc::new(Shared {
            socket,
            networked,
            id,
            key,
            running: AtomicBool::new(true),
            connected: AtomicBool::new(false),
            ptt_down: AtomicBool::new(false),
            talk: AtomicU32::new(TalkMode::Normal as u32),
            mode: AtomicU32::new(TransmitMode::PushToTalk as u32),
            muted: AtomicBool::new(false),
            deafened: AtomicBool::new(false),
            talking: AtomicBool::new(false),
            monitor: AtomicBool::new(false),
            denoise: AtomicBool::new(true),
            cave_echo: AtomicBool::new(true),
            underwater_fx: AtomicBool::new(true),
            listener_underwater: AtomicBool::new(false),
            vad_threshold: AtomicF32::new(0.02),
            mic_gain: AtomicF32::new(1.0),
            volume: AtomicF32::new(1.0),
            yaw: AtomicF32::new(0.0),
            pitch: AtomicF32::new(0.0),
            hrtf: AtomicBool::new(true),
            mic_level: AtomicF32::new(0.0),
            mic_silent: AtomicBool::new(false),
            rtt_ms: AtomicU32::new(0),
            loss_percent: AtomicU32::new(0),
            target_bitrate: AtomicU32::new(initial_bitrate),
            tx_counter: AtomicU64::new(initial_counter()),
            my_env: AtomicU32::new(0),
            group: Mutex::new(GroupState::default()),
            net_group: Mutex::new(NetGroupState::default()),
            recording: AtomicBool::new(false),
            recorded: Mutex::new(VecDeque::new()),
            peers: Mutex::new(None),
            group_volume: AtomicF32::new(1.0),
            devices: Mutex::new(DeviceSelection::default()),
            device_generation: AtomicU32::new(0),
            audio_status: AtomicU32::new(0),
            net: Mutex::new(NetState {
                policy,
                tunnel_active: networked && policy == TransportPolicy::TunnelOnly,
                tunnel_out: VecDeque::new(),
                started: Instant::now(),
                ping_sent: None,
                rx_window: ReplayWindow::default(),
                relay_window: ReplayWindow::default(),
            }),
            capture: Mutex::new(CaptureState {
                encoder,
                applied_bitrate: initial_bitrate,
                denoiser: Denoiser::new(),
                frame: Vec::with_capacity(FRAME_SAMPLES),
                seq: 0,
                hangover: 0,
                zero_run: 0,
            }),
            mixer: Mutex::new(Mixer {
                speakers: HashMap::new(),
                player_volume: HashMap::new(),
                occlusion: HashMap::new(),
                paths: HashMap::new(),
                positions: HashMap::new(),
                ready: VecDeque::with_capacity(FRAME_SAMPLES * 8),
                monitor: VecDeque::with_capacity(MONITOR_MAX),
                self_echo: VecDeque::with_capacity(MONITOR_MAX),
                reverb: Reverb::new(),
                listener_env: (0.0, 0.0),
                acoustics: Acoustics { damp: 0.35, ..Acoustics::default() },
                early: EarlyReflections::new(),
                params: SpatialParams::default(),
            }),
            // Desactivados hasta que Java mande los ajustes.
            mic_proc: Mutex::new(MicProcessor::new(false, false)),
        });

        let mut threads = Vec::new();
        if networked {
            if has_socket {
                let rx = shared.clone();
                threads.push(std::thread::Builder::new().name("ferrovoz-rx".into()).spawn(move || rx.receive_loop())?);
            }
            let ka = shared.clone();
            threads.push(std::thread::Builder::new().name("ferrovoz-keepalive".into()).spawn(move || ka.keepalive_loop())?);
        }

        if with_audio {
            #[cfg(feature = "audio")]
            threads.push(crate::audio::spawn(shared.clone())?);
            #[cfg(not(feature = "audio"))]
            log::warn!("built without the `audio` feature: no audio devices will be opened");
        }

        Ok(Self { shared, threads })
    }

    // Controles

    pub fn set_push_to_talk(&self, down: bool) {
        self.shared.ptt_down.store(down, Ordering::Relaxed);
    }
    /// Susurro, normal o grito. Afecta a lo que se envíe a partir de ahora.
    pub fn set_talk_mode(&self, talk: TalkMode) {
        self.shared.talk.store(talk as u32, Ordering::Relaxed);
    }
    pub fn set_mode(&self, mode: TransmitMode) {
        self.shared.mode.store(mode as u32, Ordering::Relaxed);
    }
    pub fn mode(&self) -> TransmitMode {
        self.shared.mode()
    }
    pub fn set_muted(&self, v: bool) {
        self.shared.muted.store(v, Ordering::Relaxed);
    }
    pub fn set_deafened(&self, v: bool) {
        self.shared.deafened.store(v, Ordering::Relaxed);
    }
    /// Monitor de micro. No se envía.
    pub fn set_monitor(&self, v: bool) {
        self.shared.monitor.store(v, Ordering::Relaxed);
        if !v {
            self.shared.mixer.lock().unwrap().monitor.clear();
        }
    }
    pub fn set_noise_suppression(&self, v: bool) {
        self.shared.denoise.store(v, Ordering::Relaxed);
    }
    /// Pitch del oyente (grados, positivo = abajo).
    pub fn set_listener_pitch(&self, pitch_deg: f32) {
        self.shared.pitch.set(pitch_deg.clamp(-90.0, 90.0));
    }

    /// HRTF (por defecto) o paneo estéreo simple, mejor para altavoces.
    pub fn set_hrtf(&self, enabled: bool) {
        self.shared.hrtf.store(enabled, Ordering::Relaxed);
    }

    pub fn set_listener_yaw(&self, yaw_deg: f32) {
        self.shared.yaw.set(yaw_deg);
    }
    pub fn set_volume(&self, v: f32) {
        self.shared.volume.set(v.clamp(0.0, 4.0));
    }
    pub fn set_mic_gain(&self, v: f32) {
        self.shared.mic_gain.set(v.clamp(0.0, 8.0));
    }
    pub fn set_vad_threshold(&self, v: f32) {
        self.shared.vad_threshold.set(v.clamp(0.0, 1.0));
    }
    /// Alcance por defecto, para paquetes que no lo traen.
    pub fn set_max_distance(&self, d: f32) {
        self.shared.mixer.lock().unwrap().params.max_distance = d.max(1.0);
    }
    /// Volumen por jugador (0.0 a 2.0).
    pub fn set_player_volume(&self, id: PlayerId, v: f32) {
        let mut m = self.shared.mixer.lock().unwrap();
        let v = v.clamp(0.0, 2.0);
        if (v - 1.0).abs() < 1e-3 {
            m.player_volume.remove(&id);
        } else {
            m.player_volume.insert(id, v);
        }
    }
    /// Oclusión de un jugador (0..1), calculada por Java.
    /// Camino alternativo para `id`, o `None` si no hay o la línea recta está libre.
    pub fn set_sound_path(&self, id: PlayerId, path: Option<SoundPath>) {
        let mut m = self.shared.mixer.lock().unwrap();
        match path {
            Some(p) if p.length.is_finite() && p.length > 0.0 => {
                m.paths.insert(id, (p, Instant::now()));
            }
            _ => {
                m.paths.remove(&id);
            }
        }
    }

    /// Acústica de la sala: `wet`, `room` (ver [`crate::reverb::rt60_for_room`]),
    /// absorción de agudos `damp` y reflexiones tempranas.
    pub fn set_acoustics(&self, wet: f32, room: f32, damp: f32, reflections: Vec<Reflection>) {
        self.set_environment(wet, room);
        let mut m = self.shared.mixer.lock().unwrap();
        m.acoustics = Acoustics {
            damp: damp.clamp(0.0, 1.0),
            reflections: limit_early_energy(reflections.into_iter().take(crate::reverb::MAX_REFLECTIONS).collect()),
        };
    }

    pub fn set_occlusion(&self, id: PlayerId, amount: f32) {
        let mut m = self.shared.mixer.lock().unwrap();
        if amount <= 0.001 {
            m.occlusion.remove(&id);
        } else {
            m.occlusion.insert(id, amount.min(1.0));
        }
    }
    /// Reverb local (`wet`, `room` en 0..1). También se envía con la voz propia.
    pub fn set_environment(&self, wet: f32, room: f32) {
        let env = Environment::from_f32(wet, room);
        self.shared.my_env.store(((env.wet as u32) << 8) | env.room as u32, Ordering::Relaxed);
        self.shared.mixer.lock().unwrap().listener_env = (wet.clamp(0.0, 1.0), room.clamp(0.0, 1.0));
    }

    /// Reverb en cuevas y salas (por defecto activada).
    pub fn set_cave_echo(&self, enabled: bool) {
        self.shared.cave_echo.store(enabled, Ordering::Relaxed);
    }

    /// Efecto submarino (por defecto activado).
    pub fn set_underwater_effect(&self, enabled: bool) {
        self.shared.underwater_fx.store(enabled, Ordering::Relaxed);
    }

    /// AEC y AGC del micro.
    pub fn set_mic_processing(&self, echo_cancel: bool, auto_gain: bool) {
        self.shared.mic_proc.lock().unwrap().configure(echo_cancel, auto_gain);
    }

    /// Avisa al servidor de que se deja la voz para que deje de enviar audio.
    /// Llamar antes de `stop`.
    pub fn goodbye(&self) {
        if self.shared.networked {
            self.shared.send(self.shared.seal(PacketType::Bye, &[]));
            self.shared.connected.store(false, Ordering::Relaxed);
        }
    }

    /// Cabeza bajo el agua (Java lo comprueba cada tick).
    pub fn set_underwater(&self, underwater: bool) {
        self.shared.listener_underwater.store(underwater, Ordering::Relaxed);
    }

    /// Grupo actual (nombre vacío = ninguno).
    pub fn group_state(&self) -> GroupState {
        self.shared.group.lock().unwrap().clone()
    }

    /// Volumen de la voz de grupo.
    pub fn set_group_volume(&self, v: f32) {
        self.shared.group_volume.set(v.clamp(0.0, 2.0));
    }

    /// Estado de voz de los jugadores: (UUID, flags de [`PeerStates`]).
    /// `None` si el servidor no lo envía.
    pub fn peer_states(&self) -> Option<Vec<(PlayerId, u8)>> {
        let now = Instant::now();
        self.shared.peers.lock().unwrap().as_ref().map(|m| {
            m.iter().filter(|(_, (_, t))| now.duration_since(*t) < PEER_STATE_TTL).map(|(id, (f, _))| (*id, *f)).collect()
        })
    }

    // Repeticiones (Replay Mod, Flashback)

    /// Activa la grabación de tramas.
    pub fn set_recording(&self, on: bool) {
        self.shared.recording.store(on, Ordering::Relaxed);
        if !on {
            self.shared.recorded.lock().unwrap().clear();
        }
    }

    /// Tramas grabadas desde la última llamada (cuerpos de `AudioDown`).
    pub fn poll_recorded(&self) -> Vec<Vec<u8>> {
        self.shared.recorded.lock().unwrap().drain(..).collect()
    }

    /// Reproduce una trama grabada como si llegara por la red.
    pub fn replay_feed(&self, audio_down: &[u8]) -> bool {
        match AudioDown::decode(audio_down) {
            Ok(frame) => {
                self.shared.play_audio_down(frame);
                true
            }
            Err(_) => false,
        }
    }

    /// Posición de `id` relativa a la cámara de la repetición.
    pub fn set_source_position(&self, id: PlayerId, rel: [f32; 3]) {
        if rel.iter().all(|v| v.is_finite()) {
            self.shared.mixer.lock().unwrap().positions.insert(id, (rel, Instant::now()));
        }
    }

    /// Grupo de red (nombre vacío = ninguno).
    pub fn net_group(&self) -> NetGroupState {
        self.shared.net_group.lock().unwrap().clone()
    }

    /// Cambia de dispositivo en caliente. Cadena vacía = predeterminado.
    pub fn set_devices(&self, input: &str, output: &str) {
        let mut d = self.shared.devices.lock().unwrap();
        let new = DeviceSelection { input: input.to_owned(), output: output.to_owned() };
        if *d != new {
            *d = new;
            self.shared.device_generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    // Túnel

    /// Paquete recibido por la conexión del juego.
    pub fn handle_tunnel(&self, data: &[u8]) {
        self.shared.handle_incoming(data);
    }

    /// Paquetes pendientes de enviar por la conexión del juego.
    pub fn poll_tunnel(&self) -> Vec<Vec<u8>> {
        self.shared.net.lock().unwrap().tunnel_out.drain(..).collect()
    }

    // Estado para la UI

    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed)
    }
    pub fn is_talking(&self) -> bool {
        self.shared.talking.load(Ordering::Relaxed)
    }
    /// RMS del micro en la última trama (0..1), después del procesado.
    /// Lleva 3 s recibiendo ceros (permiso denegado o mute por hardware).
    pub fn mic_silent(&self) -> bool {
        self.shared.mic_silent.load(Ordering::Relaxed)
    }

    pub fn mic_level(&self) -> f32 {
        self.shared.mic_level.get()
    }
    /// 0 = abriendo, 1 = abiertos, 2 = error.
    pub fn audio_status(&self) -> u32 {
        self.shared.audio_status.load(Ordering::Relaxed)
    }
    pub fn net_stats(&self) -> NetStats {
        NetStats {
            rtt_ms: self.shared.rtt_ms.load(Ordering::Relaxed),
            loss_percent: self.shared.loss_percent.load(Ordering::Relaxed) as u8,
            bitrate: self.shared.target_bitrate.load(Ordering::Relaxed),
            tunnel: self.shared.net.lock().unwrap().tunnel_active,
        }
    }
    /// Jugadores que están hablando ahora, y si es por grupo.
    /// Está llegando el murmullo de una multitud (ver [`crate::crowd`]).
    pub fn hearing_crowd(&self) -> bool {
        let now = Instant::now();
        self.shared
            .mixer
            .lock()
            .unwrap()
            .speakers
            .get(&protocol::CROWD_ID)
            .is_some_and(|s| now.duration_since(s.last_packet) < SPEAKING_WINDOW)
    }

    pub fn speaking_players(&self) -> Vec<(PlayerId, bool)> {
        let now = Instant::now();
        self.shared
            .mixer
            .lock()
            .unwrap()
            .speakers
            .iter()
            .filter(|(id, s)| now.duration_since(s.last_packet) < SPEAKING_WINDOW && **id != protocol::CROWD_ID)
            .map(|(id, s)| (*id, s.group))
            .collect()
    }

    // E/S de audio (la usan `audio` y los tests)

    pub fn push_capture(&self, mono_48k: &[f32]) {
        self.shared.push_capture(mono_48k)
    }
    pub fn render(&self, stereo_48k: &mut [f32]) {
        self.shared.render(stereo_48k)
    }

    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for VoiceClient {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    // Solo para el hilo de audio y los tests.
    #[cfg_attr(not(feature = "audio"), allow(dead_code))]
    pub(crate) fn device_generation(&self) -> u32 {
        self.device_generation.load(Ordering::SeqCst)
    }

    #[cfg_attr(not(feature = "audio"), allow(dead_code))]
    pub(crate) fn devices(&self) -> DeviceSelection {
        self.devices.lock().unwrap().clone()
    }

    fn mode(&self) -> TransmitMode {
        if self.mode.load(Ordering::Relaxed) == TransmitMode::VoiceActivation as u32 {
            TransmitMode::VoiceActivation
        } else {
            TransmitMode::PushToTalk
        }
    }

    /// Cifra con el siguiente contador.
    fn seal(&self, kind: PacketType, body: &[u8]) -> Vec<u8> {
        let counter = self.tx_counter.fetch_add(1, Ordering::Relaxed) + 1;
        protocol::seal(&self.key, kind, Some(&self.id), body, counter)
    }

    /// Envía por el transporte activo.
    fn send(&self, pkt: Vec<u8>) {
        let mut net = self.net.lock().unwrap();
        if net.tunnel_active {
            if net.tunnel_out.len() >= TUNNEL_QUEUE_MAX {
                net.tunnel_out.pop_front();
            }
            net.tunnel_out.push_back(pkt);
        } else if let Some(s) = &self.socket {
            drop(net);
            let _ = s.send(&pkt);
        }
    }

    fn keepalive_loop(&self) {
        let mut last = Instant::now() - Duration::from_secs(60);
        let mut last_state: Option<(u8, Instant)> = None;
        while self.is_running() {
            let connected = self.connected.load(Ordering::Relaxed);
            // Estado de voz: al cambiar y cada 5 s.
            if connected {
                let flags = (if self.muted.load(Ordering::Relaxed) { PeerStates::MUTED } else { 0 })
                    | (if self.deafened.load(Ordering::Relaxed) { PeerStates::DEAFENED } else { 0 });
                if last_state.is_none_or(|(f, t)| f != flags || t.elapsed() >= VOICE_STATE_EVERY) {
                    self.send(self.seal(PacketType::VoiceState, &[flags]));
                    last_state = Some((flags, Instant::now()));
                }
            } else {
                last_state = None;
            }
            {
                // Sin respuesta por UDP: túnel, si se permite.
                let mut net = self.net.lock().unwrap();
                if !connected
                    && !net.tunnel_active
                    && net.policy == TransportPolicy::Auto
                    && net.started.elapsed() >= UDP_FALLBACK_AFTER
                {
                    net.tunnel_active = true;
                    last = Instant::now() - Duration::from_secs(60); // Hello inmediato por el túnel
                    log::info!("no UDP response: voice now goes through the game connection (tunnel)");
                }
            }
            // Hello cada 500 ms hasta tener respuesta; después ping cada 2 s (RTT).
            let interval = if connected { Duration::from_secs(2) } else { Duration::from_millis(500) };
            if last.elapsed() >= interval {
                let kind = if connected { PacketType::Ping } else { PacketType::Hello };
                self.net.lock().unwrap().ping_sent = Some(Instant::now());
                self.send(self.seal(kind, &[]));
                last = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn receive_loop(&self) {
        let Some(socket) = &self.socket else { return };
        let mut buf = [0u8; MAX_PACKET];
        while self.is_running() {
            if let Ok(n) = socket.recv(&mut buf) {
                self.handle_incoming(&buf[..n]);
            }
        }
    }

    fn handle_incoming(&self, data: &[u8]) {
        let Ok(raw) = protocol::parse(data) else { return };
        // Anti-replay.
        let Ok(body) = ({
            let mut net = self.net.lock().unwrap();
            // Los paquetes del proxy llevan su propio contador.
            let window = if raw.kind.direction() == protocol::Direction::Relay {
                &mut net.relay_window
            } else {
                &mut net.rx_window
            };
            raw.open_once(&self.key, window)
        }) else {
            return;
        };
        match raw.kind {
            PacketType::Pong => {
                self.connected.store(true, Ordering::Relaxed);
                if let Some(sent) = self.net.lock().unwrap().ping_sent.take() {
                    self.rtt_ms.store(sent.elapsed().as_millis().min(10_000) as u32, Ordering::Relaxed);
                }
                // Ajuste de bitrate y FEC según la pérdida.
                let loss = PongInfo::decode(&body).loss_percent;
                self.loss_percent.store(loss as u32, Ordering::Relaxed);
                self.target_bitrate.store(bitrate_for_loss(loss), Ordering::Relaxed);
            }
            PacketType::GroupState => {
                if let Ok(g) = GroupState::decode(&body) {
                    *self.group.lock().unwrap() = g;
                }
            }
            PacketType::PeerStates => {
                if let Ok(states) = PeerStates::decode(&body) {
                    let now = Instant::now();
                    let mut peers = self.peers.lock().unwrap();
                    let map = peers.get_or_insert_with(HashMap::new);
                    map.retain(|_, (_, t)| now.duration_since(*t) < PEER_STATE_TTL);
                    for (id, flags) in states.entries {
                        map.insert(id, (flags, now));
                    }
                }
            }
            PacketType::RelayGroup => {
                if let Ok(g) = NetGroupState::decode(&body) {
                    *self.net_group.lock().unwrap() = g;
                }
            }
            PacketType::AudioDown | PacketType::RelayAudio => {
                self.record(&body);
                if let Ok(frame) = AudioDown::decode(&body) {
                    self.play_audio_down(frame);
                }
            }
            _ => {}
        }
    }

    /// Guarda la trama si hay grabación activa.
    fn record(&self, audio_down: &[u8]) {
        if self.recording.load(Ordering::Relaxed) {
            let mut q = self.recorded.lock().unwrap();
            if q.len() >= RECORDED_MAX {
                q.pop_front();
            }
            q.push_back(audio_down.to_vec());
        }
    }

    /// Trama recibida: al búfer de su hablante.
    fn play_audio_down(&self, frame: AudioDown) {
        let mut mixer = self.mixer.lock().unwrap();
        let speaker = match mixer.speakers.entry(frame.sender) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => match VoiceDecoder::new() {
                Ok(decoder) => e.insert(Speaker {
                    jitter: JitterBuffer::new(),
                    decoder,
                    rel: frame.rel,
                    range: frame.range,
                    group: frame.group,
                    last_packet: Instant::now(),
                    occlusion: 0.0,
                    server_occlusion: None,
                    env: frame.env,
                    lowpass: 0.0,
                    underwater: false,
                    water: Underwater::new(),
                    binaural: Binaural::new(),
                    path_binaural: Binaural::new(),
                    path_lowpass: 0.0,
                    path_gain: 0.0,
                }),
                Err(_) => return,
            },
        };
        speaker.rel = frame.rel;
        speaker.range = frame.range;
        speaker.group = frame.group;
        speaker.env = frame.env;
        speaker.underwater = frame.underwater;
        speaker.server_occlusion =
            (frame.occlusion != OCCLUSION_UNKNOWN).then(|| frame.occlusion as f32 / 254.0);
        speaker.last_packet = Instant::now();
        speaker.jitter.push(frame.seq, frame.opus);
    }

    pub(crate) fn push_capture(&self, input: &[f32]) {
        let gain = self.mic_gain.get();
        let mut cap = self.capture.lock().unwrap();
        if input.iter().all(|&s| s == 0.0) {
            cap.zero_run = cap.zero_run.saturating_add(input.len());
        } else {
            cap.zero_run = 0;
        }
        self.mic_silent.store(cap.zero_run >= MIC_SILENT_SAMPLES, Ordering::Relaxed);
        for &s in input {
            cap.frame.push((s * gain).clamp(-1.0, 1.0));
            if cap.frame.len() == FRAME_SAMPLES {
                self.process_frame(&mut cap);
                cap.frame.clear();
            }
        }
    }

    fn process_frame(&self, cap: &mut CaptureState) {
        // AEC -> RNNoise -> AGC. La ganancia va al final para no amplificar el ruido que
        // se cuela (con cascos, el sonido del juego).
        self.mic_proc.lock().unwrap().cancel_echo(&mut cap.frame);
        let (prob, open) = if self.denoise.load(Ordering::Relaxed) {
            let CaptureState { denoiser, frame, .. } = cap;
            let p = denoiser.process(frame);
            (Some(p), denoiser.voice_open())
        } else {
            (None, 1.0)
        };
        self.mic_proc.lock().unwrap().adjust_gain(&mut cap.frame, prob, open);

        let rms = (cap.frame.iter().map(|s| s * s).sum::<f32>() / FRAME_SAMPLES as f32).sqrt();
        self.mic_level.set(rms);

        if self.monitor.load(Ordering::Relaxed) {
            let mut mixer = self.mixer.lock().unwrap();
            mixer.monitor.extend(cap.frame.iter().copied());
            let excess = mixer.monitor.len().saturating_sub(MONITOR_MAX);
            mixer.monitor.drain(..excess);
        }

        let send = if self.muted.load(Ordering::Relaxed) || self.deafened.load(Ordering::Relaxed) {
            cap.hangover = 0;
            false
        } else {
            match self.mode() {
                TransmitMode::PushToTalk => self.ptt_down.load(Ordering::Relaxed),
                TransmitMode::VoiceActivation => {
                    if rms >= self.vad_threshold.get() {
                        cap.hangover = VAD_HANGOVER_FRAMES;
                    } else {
                        cap.hangover = cap.hangover.saturating_sub(1);
                    }
                    cap.hangover > 0
                }
            }
        };
        self.talking.store(send, Ordering::Relaxed);

        // Voz propia hacia la reverb (solo la parte húmeda).
        if send && self.cave_echo.load(Ordering::Relaxed) {
            let mut mixer = self.mixer.lock().unwrap();
            if mixer.listener_env.0 > 0.0 {
                mixer.self_echo.extend(cap.frame.iter().copied());
                let excess = mixer.self_echo.len().saturating_sub(MONITOR_MAX);
                mixer.self_echo.drain(..excess);
            }
        }

        if !self.networked || !send || !self.connected.load(Ordering::Relaxed) {
            return;
        }

        // Bitrate adaptativo.
        let target = self.target_bitrate.load(Ordering::Relaxed);
        if target != cap.applied_bitrate {
            let loss = self.loss_percent.load(Ordering::Relaxed) as i32;
            cap.encoder.tune(target as i32, loss.max(5));
            cap.applied_bitrate = target;
        }

        let seq = cap.seq;
        cap.seq = cap.seq.wrapping_add(1);
        let talk = TalkMode::from_u8(self.talk.load(Ordering::Relaxed) as u8);
        let packed = self.my_env.load(Ordering::Relaxed);
        let env = Environment { wet: (packed >> 8) as u8, room: packed as u8 };
        let CaptureState { encoder, frame, .. } = cap;
        if let Ok(opus) = encoder.encode(frame).map(|b| b.to_vec()) {
            if self.recording.load(Ordering::Relaxed) {
                // La voz propia también se graba, en la posición del jugador.
                let normal = self.mixer.lock().unwrap().params.max_distance;
                let range = match talk {
                    TalkMode::Whisper => normal / 6.0,
                    TalkMode::Shout => normal * 2.0,
                    TalkMode::Normal => normal,
                };
                let group = !self.group.lock().unwrap().name.is_empty() || !self.net_group.lock().unwrap().name.is_empty();
                let mine = AudioDown {
                    sender: self.id,
                    seq,
                    rel: [0.0; 3],
                    range: if group { 0.0 } else { range },
                    group,
                    underwater: self.listener_underwater.load(Ordering::Relaxed),
                    env,
                    occlusion: OCCLUSION_UNKNOWN,
                    opus: opus.clone(),
                };
                self.record(&mine.encode());
            }
            let body = AudioUp { seq, talk, env, opus }.encode();
            self.send(self.seal(PacketType::AudioUp, &body));
        }
    }

    pub(crate) fn render(&self, out: &mut [f32]) {
        let mut mixer = self.mixer.lock().unwrap();
        while mixer.ready.len() < out.len() {
            self.mix_frame(&mut mixer);
        }
        for o in out.iter_mut() {
            *o = mixer.ready.pop_front().unwrap_or(0.0);
        }
    }

    fn mix_frame(&self, mixer: &mut Mixer) {
        let mut mix = [0f32; FRAME_SAMPLES * 2];
        let mut reverb_in = [0f32; FRAME_SAMPLES];
        let mut pcm = [0f32; FRAME_SAMPLES];
        let yaw = self.yaw.get();
        let pitch = self.pitch.get();
        let hrtf = self.hrtf.load(Ordering::Relaxed);
        let deaf = self.deafened.load(Ordering::Relaxed);
        let volume = if deaf { 0.0 } else { self.volume.get() };
        let group_volume = if deaf { 0.0 } else { self.group_volume.get() };
        let base = mixer.params;
        let now = Instant::now();

        mixer.speakers.retain(|_, s| now.duration_since(s.last_packet) < SPEAKER_TIMEOUT);

        let cave_echo = self.cave_echo.load(Ordering::Relaxed);
        let water_fx = self.underwater_fx.load(Ordering::Relaxed);
        let me_underwater = water_fx && self.listener_underwater.load(Ordering::Relaxed);
        let (listener_wet, listener_room) = mixer.listener_env;
        let mut room = listener_room;
        mixer.paths.retain(|_, (_, t)| now.duration_since(*t) < PATH_TTL);
        mixer.positions.retain(|_, (_, t)| now.duration_since(*t) < POSITION_TTL);
        let Mixer { speakers, player_volume, occlusion, paths, positions, .. } = mixer;
        for (id, speaker) in speakers.iter_mut() {
            if let Some((rel, _)) = positions.get(id) {
                speaker.rel = *rel;
            }
            let produced = match speaker.jitter.pop() {
                Pop::Frame(pkt) => speaker.decoder.decode(&pkt, &mut pcm),
                Pop::Lost { next: Some(next) } => speaker.decoder.decode_fec(&next, &mut pcm),
                Pop::Lost { next: None } => speaker.decoder.conceal(&mut pcm),
                Pop::Idle => 0,
            };
            if produced == 0 {
                continue;
            }
            let n = produced.min(FRAME_SAMPLES);

            // Efecto submarino si el oyente o el hablante están sumergidos. Se aplica
            // también a la voz de grupo.
            let them_underwater = water_fx && speaker.underwater;
            let water_target = match (me_underwater, them_underwater) {
                (true, true) => underwater::BOTH,
                (false, false) => 0.0,
                _ => underwater::ONE_SIDE,
            };
            speaker.water.process(&mut pcm[..n], water_target);

            let personal = player_volume.get(id).copied().unwrap_or(1.0);
            let v = volume * personal;

            if speaker.group {
                // Voz de grupo: centrada, sin distancia ni oclusión.
                let gv = group_volume * personal;
                for i in 0..n {
                    mix[2 * i] += pcm[i] * GROUP_GAIN * gv;
                    mix[2 * i + 1] += pcm[i] * GROUP_GAIN * gv;
                }
                continue;
            }
            if v <= 0.0 {
                continue;
            }

            // Oclusión: la de Java si ve al hablante, si no la del servidor. Suavizada.
            let target = occlusion.get(id).copied().or(speaker.server_occlusion).unwrap_or(0.0);
            speaker.occlusion += (target - speaker.occlusion) * 0.3;
            let occ = speaker.occlusion;
            // Paso bajo de un polo: ~20 kHz sin oclusión, ~500 Hz muy tapado.
            let cutoff = 20_000.0 * (500.0f32 / 20_000.0).powf(occ);
            let a = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / SAMPLE_RATE as f32).exp();
            let occ_gain = 1.0 - 0.65 * occ;

            let range = if speaker.range > 0.0 { speaker.range } else { base.max_distance };
            let params = SpatialParams {
                max_distance: range,
                full_volume_distance: base.full_volume_distance.min(range * 0.25),
                ..base
            };
            // Reverb: la del oyente o la del hablante, la mayor.
            let send = if cave_echo { listener_wet.max(speaker.env.wet()) } else { 0.0 };

            // Camino alternativo: si la línea recta está tapada, la voz llega también
            // desde el hueco, más baja cuanto más largo el camino y más esquinas.
            let path_target = match paths.get(id) {
                Some((p, _)) if occ > 0.05 => {
                    spatial::distance_gain(p.length, &params) * v * CORNER_GAIN.powi(p.corners.min(6) as i32)
                }
                _ => 0.0,
            };
            speaker.path_gain += (path_target - speaker.path_gain) * 0.3;
            if speaker.path_gain > 0.001 {
                if let Some((p, _)) = paths.get(id) {
                    let rel = [p.dir[0] * p.length, p.dir[1] * p.length, p.dir[2] * p.length];
                    let cut = CORNER_CUTOFF / (1.0 + p.corners as f32);
                    let pa = 1.0 - (-2.0 * std::f32::consts::PI * cut / SAMPLE_RATE as f32).exp();
                    let g = speaker.path_gain;
                    let mut dry = [0f32; FRAME_SAMPLES];
                    for i in 0..n {
                        speaker.path_lowpass += pa * (pcm[i] - speaker.path_lowpass);
                        dry[i] = speaker.path_lowpass;
                        reverb_in[i] += dry[i] * g * send;
                    }
                    if hrtf {
                        let dir = Direction::from_world(rel, yaw, pitch).unwrap_or(Direction::FRONT);
                        speaker.path_binaural.process_add(&dry[..n], dir, g, &mut mix);
                    } else {
                        // La distancia ya está en `g`.
                        let (pl, pr) = spatial::stereo_gains(p.dir, yaw, &SpatialParams { full_volume_distance: 2.0, ..params });
                        for i in 0..n {
                            mix[2 * i] += dry[i] * pl * g;
                            mix[2 * i + 1] += dry[i] * pr * g;
                        }
                    }
                }
            }

            if hrtf {
                let r = speaker.rel;
                let dist = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
                let g = spatial::distance_gain(dist, &params) * v * occ_gain;
                if g <= 0.0 {
                    continue;
                }
                if cave_echo && speaker.env.wet() > 0.0 {
                    room = room.max(speaker.env.room());
                }
                // Demasiado cerca para tener dirección: de frente.
                let dir = Direction::from_world(r, yaw, pitch).unwrap_or(Direction::FRONT);
                let mut dry = [0f32; FRAME_SAMPLES];
                for i in 0..n {
                    speaker.lowpass += a * (pcm[i] - speaker.lowpass);
                    dry[i] = speaker.lowpass;
                    reverb_in[i] += dry[i] * g * send;
                }
                speaker.binaural.process_add(&dry[..n], dir, g, &mut mix);
                continue;
            }

            let (gl, gr) = spatial::stereo_gains(speaker.rel, yaw, &params);
            let (gl, gr) = (gl * v * occ_gain, gr * v * occ_gain);
            if gl + gr <= 0.0 {
                continue;
            }
            if cave_echo && speaker.env.wet() > 0.0 {
                room = room.max(speaker.env.room());
            }
            for i in 0..n {
                speaker.lowpass += a * (pcm[i] - speaker.lowpass);
                let s = speaker.lowpass;
                mix[2 * i] += s * gl;
                mix[2 * i + 1] += s * gr;
                reverb_in[i] += s * (gl + gr) * 0.5 * send;
            }
        }

        // Voz propia: solo a la cola de la reverb. Llega con unos ms de retraso y en
        // las reflexiones tempranas sonaría duplicada. En salas pequeñas no se aplica.
        let mut self_reverb = [0f32; FRAME_SAMPLES];
        if !mixer.self_echo.is_empty() {
            let big = ((listener_room - SELF_ECHO_MIN_ROOM) / SELF_ECHO_MIN_ROOM).clamp(0.0, 1.0);
            let gain = if cave_echo { listener_wet * SELF_ECHO_GAIN * volume * big * big * (3.0 - 2.0 * big) } else { 0.0 };
            for slot in self_reverb.iter_mut() {
                let Some(s) = mixer.self_echo.pop_front() else { break };
                *slot = s * gain;
            }
        }

        // Reflexiones tempranas, giradas con la cabeza del oyente.
        let early: Vec<(f32, f32, f32, f32)> = if cave_echo {
            mixer
                .acoustics
                .reflections
                .iter()
                .filter_map(|r| {
                    let d = Direction::from_world([r.dir[0] * 8.0, r.dir[1] * 8.0, r.dir[2] * 8.0], yaw, pitch)?;
                    Some((d.x, d.z, r.delay, r.gain * EARLY_LEVEL))
                })
                .collect()
        } else {
            Vec::new()
        };
        if !early.is_empty() || mixer.early.is_active() {
            mixer.early.process_add(&early, &reverb_in, &mut mix);
        }

        // Reverb de la sala sobre las voces de proximidad y la propia.
        for (r, s) in reverb_in.iter_mut().zip(self_reverb) {
            *r += s;
        }
        if reverb_in.iter().any(|s| *s != 0.0) {
            mixer.reverb.set_params(1.0, room);
            mixer.reverb.set_damp(mixer.acoustics.damp);
            mixer.reverb.process_add(&reverb_in, &mut mix);
        } else if mixer.reverb.is_active() {
            // Deja terminar la cola.
            mixer.reverb.process_add(&reverb_in, &mut mix);
        }

        // Referencia del AEC: todo lo que suena menos el monitor de micro (si no, el
        // AEC quitaría la propia voz en la prueba).
        let mut reference = [0f32; FRAME_SAMPLES];
        for (i, r) in reference.iter_mut().enumerate() {
            *r = 0.5 * (mix[2 * i].tanh() + mix[2 * i + 1].tanh());
        }

        // Monitor de micro: centrado, sin efectos.
        if !mixer.monitor.is_empty() {
            for i in 0..FRAME_SAMPLES {
                let Some(s) = mixer.monitor.pop_front() else { break };
                mix[2 * i] += s;
                mix[2 * i + 1] += s;
            }
        }

        // Limitador suave.
        mixer.ready.extend(mix.iter().map(|s| s.tanh()));
        self.mic_proc.lock().unwrap().render(&reference);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::VoiceEncoder;

    fn tone(n: usize, freq: f32) -> Vec<f32> {
        (0..n).map(|i| 0.4 * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin()).collect()
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|s| s * s).sum()
    }

    /// Inyecta voz como si llegara por la red.
    fn feed(c: &VoiceClient, sender: PlayerId, rel: [f32; 3], group: bool, freq: f32, frames: usize) -> Vec<f32> {
        let mut enc = VoiceEncoder::new(32_000).unwrap();
        let mut rendered = Vec::new();
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        let signal = tone(FRAME_SAMPLES * frames, freq);
        for f in 0..frames {
            let opus = enc.encode(&signal[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES]).unwrap().to_vec();
            let down = AudioDown {
                sender,
                seq: f as u32,
                rel,
                range: 48.0,
                group,
                underwater: false,
                env: Environment::default(),
                occlusion: OCCLUSION_UNKNOWN,
                opus,
            };
            let pkt = protocol::seal(&c.shared.key, PacketType::AudioDown, None, &down.encode(), next_counter());
            c.handle_tunnel(&pkt);
            c.render(&mut out);
            if f >= 6 {
                rendered.extend_from_slice(&out);
            }
        }
        rendered
    }

    fn next_counter() -> u64 {
        static C: AtomicU64 = AtomicU64::new(1);
        C.fetch_add(1, Ordering::Relaxed)
    }

    fn local() -> VoiceClient {
        VoiceClient::start_local(false).unwrap()
    }

    #[test]
    fn monitor_plays_your_own_voice_locally() {
        let c = local();
        c.set_noise_suppression(false);
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        c.push_capture(&tone(FRAME_SAMPLES, 300.0));
        c.render(&mut out);
        assert_eq!(energy(&out), 0.0, "sin prueba de micro no se oye nada");
        c.set_monitor(true);
        c.push_capture(&tone(FRAME_SAMPLES, 300.0));
        c.render(&mut out);
        assert!(energy(&out) > 10.0, "con prueba de micro te oyes");
        assert!((out[0] - out[1]).abs() < 1e-6);
    }

    /// Con altavoces el micro capta a los demás. Sin AEC la activación por voz salta
    /// y se reenvía su voz; con AEC no.
    #[cfg(feature = "audio")]
    #[test]
    fn with_speakers_echo_cancel_stops_you_from_resending_others() {
        let talking_frames = |echo_cancel: bool| {
            let c = local();
            c.set_noise_suppression(false);
            c.set_mode(TransmitMode::VoiceActivation);
            c.set_vad_threshold(0.02);
            c.set_mic_processing(echo_cancel, false);
            let other = crate::micproc::tests::voice_like(FRAME_SAMPLES * 300, 5);
            let mut enc = VoiceEncoder::new(32_000).unwrap();
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            // Altavoz -> micro: 40 ms y atenuado.
            let mut air: VecDeque<f32> = std::iter::repeat_n(0.0, 48 * 40).collect();
            let mut talking = 0;
            for f in 0..300 {
                let opus = enc.encode(&other[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES]).unwrap().to_vec();
                let down = AudioDown {
                    sender: [9; 16],
                    seq: f as u32,
                    rel: [0.0, 0.0, 2.0],
                    range: 48.0,
                    group: true,
                    underwater: false,
                    env: Environment::default(),
                    occlusion: OCCLUSION_UNKNOWN,
                    opus,
                };
                let pkt = protocol::seal(&c.shared.key, PacketType::AudioDown, None, &down.encode(), next_counter());
                c.handle_tunnel(&pkt);
                c.render(&mut out);
                air.extend(out.chunks(2).map(|lr| 0.6 * (lr[0] + lr[1])));
                let mic: Vec<f32> = air.drain(..FRAME_SAMPLES).collect();
                c.push_capture(&mic);
                if f >= 150 && c.is_talking() {
                    talking += 1;
                }
            }
            talking
        };
        let without = talking_frames(false);
        let with = talking_frames(true);
        eprintln!("tramas reenviadas: sin cancelación {without}, con cancelación {with}");
        assert!(without > 100, "la prueba no reproduce el problema ({without})");
        assert!(with < 10, "con cancelación aún se dispara {with} veces");
    }

    /// Con cascos se cuela algo del juego en el micro y el AEC no lo puede quitar.
    /// El AGC no debe subirlo en las pausas.
    #[cfg(feature = "audio")]
    #[test]
    fn auto_gain_does_not_boost_headphone_leak() {
        // 2 s hablando bajo y 2 s callado, cuatro veces, con fondo de ruido filtrado a
        // unos -50 dBFS.
        let voice = crate::micproc::tests::voice_like(FRAME_SAMPLES * 400, 4);
        let measure = |auto_gain: bool| {
            let c = local();
            c.set_mic_processing(false, auto_gain);
            let mut x = 5u32;
            let mut lp = 0f32;
            let (mut pause, mut pause_n, mut talk, mut talk_n) = (0f32, 0, 0f32, 0);
            for f in 0..400 {
                let talking = (f / 100) % 2 == 0;
                let frame: Vec<f32> = (0..FRAME_SAMPLES)
                    .map(|k| {
                        x ^= x << 13;
                        x ^= x >> 17;
                        x ^= x << 5;
                        lp = 0.8 * lp + 0.2 * (x as f32 / u32::MAX as f32 * 2.0 - 1.0);
                        // Acorde que va y viene más algo de ruido.
                        let t = (f * FRAME_SAMPLES + k) as f32 / SAMPLE_RATE as f32;
                        let chord: f32 = [220.0f32, 277.0, 330.0, 440.0]
                            .iter()
                            .map(|hz| (2.0 * std::f32::consts::PI * hz * t).sin())
                            .sum();
                        let leak = chord * 0.004 * (0.6 + 0.4 * (t * 1.3).sin()) + lp * 0.004;
                        leak + if talking { voice[f * FRAME_SAMPLES + k] * 0.15 } else { 0.0 }
                    })
                    .collect();
                c.push_capture(&frame);
                let level = c.mic_level();
                if f >= 200 && !talking && f % 100 >= 25 {
                    pause += level * level;
                    pause_n += 1;
                } else if f >= 200 && talking {
                    talk += level * level;
                    talk_n += 1;
                }
            }
            (10.0 * (pause / pause_n as f32).log10(), 10.0 * (talk / talk_n as f32).log10())
        };
        let (pause_off, talk_off) = measure(false);
        let (pause_on, talk_on) = measure(true);
        println!(
            "pausas: {pause_off:.1} → {pause_on:.1} dBFS · hablando: {talk_off:.1} → {talk_on:.1} dBFS"
        );
        // El fondo no puede subir más que la voz, y la voz sí sube.
        assert!(
            (pause_on - pause_off) <= (talk_on - talk_off) + 1.0,
            "lo de los cascos sube {:.1} dB y tu voz {:.1} dB",
            pause_on - pause_off,
            talk_on - talk_off
        );
        assert!(talk_on - talk_off > 6.0, "pero sigue subiendo la voz baja");
    }

    /// Monitor de micro con cascos: el AEC no debe quitar la voz propia.
    #[cfg(feature = "audio")]
    #[test]
    fn mic_test_with_headphones_is_not_cancelled() {
        let heard = |echo_cancel: bool| {
            let c = local();
            c.set_noise_suppression(false);
            c.set_mic_processing(echo_cancel, false);
            c.set_monitor(true);
            let voice = crate::micproc::tests::voice_like(FRAME_SAMPLES * 250, 3);
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            let mut e = 0f32;
            for (f, frame) in voice.chunks_exact(FRAME_SAMPLES).enumerate() {
                c.push_capture(frame);
                c.render(&mut out);
                if f >= 100 {
                    e += energy(&out);
                }
            }
            e
        };
        let (off, on) = (heard(false), heard(true));
        let db = 10.0 * (on / off).log10();
        eprintln!("prueba de micro con cancelación: {db:+.1} dB");
        assert!(db > -3.0, "tu voz baja {db:.1} dB");
    }

    #[test]
    fn blocked_microphone_is_detected_but_a_quiet_room_is_not() {
        let c = local();
        let zeros = vec![0f32; FRAME_SAMPLES];
        // 2.9 s de ceros: todavía no.
        for _ in 0..145 {
            c.push_capture(&zeros);
        }
        assert!(!c.mic_silent());
        for _ in 0..10 {
            c.push_capture(&zeros);
        }
        assert!(c.mic_silent(), "3 s de ceros exactos = micro bloqueado");
        // Ruido de fondo mínimo: no cuenta como bloqueado.
        let hiss: Vec<f32> = (0..FRAME_SAMPLES).map(|i| if i % 7 == 0 { 1e-6 } else { -1e-6 }).collect();
        c.push_capture(&hiss);
        assert!(!c.mic_silent(), "en cuanto llega algo de señal, se quita el aviso");
        for _ in 0..200 {
            c.push_capture(&hiss);
        }
        assert!(!c.mic_silent());
    }

    #[test]
    fn in_a_cave_you_hear_your_own_echo_but_not_your_dry_voice() {
        // Energía mientras habla (40 tramas) y cola al callar (40 más).
        let run = |wet: f32, echo: bool, talking: bool| {
            let c = local();
            c.set_noise_suppression(false);
            c.set_environment(wet, 1.0);
            c.set_cave_echo(echo);
            c.set_push_to_talk(talking);
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            let (mut first, mut during, mut tail) = (0f32, 0f32, 0f32);
            for n in 0..40 {
                c.push_capture(&tone(FRAME_SAMPLES, 300.0));
                c.render(&mut out);
                if n == 0 {
                    first = energy(&out);
                }
                during += energy(&out);
            }
            c.set_push_to_talk(false);
            for _ in 0..40 {
                c.push_capture(&vec![0.0; FRAME_SAMPLES]);
                c.render(&mut out);
                tail += energy(&out);
            }
            (first, during, tail)
        };
        let (first, during, tail) = run(0.45, true, true);
        let dry_frame = energy(&tone(FRAME_SAMPLES, 300.0)) * 2.0; // voz seca en estéreo
        println!("tu eco en la cueva: primera trama {first:.4} (voz seca {dry_frame:.1}), hablando {during:.2}, cola {tail:.2}");
        assert!(first < dry_frame * 0.01, "tu voz seca no suena: solo el eco, que llega después");
        assert!(during > 1.0 && tail > 0.1, "te oyes rebotar y la cola sigue al callar");
        assert_eq!(run(0.45, false, true).1, 0.0, "con el eco desactivado, nada");
        assert_eq!(run(0.0, true, true).1, 0.0, "fuera de la cueva, nada");
        assert_eq!(run(0.45, true, false).1, 0.0, "si no transmites, nada");
    }

    #[test]
    fn monitor_latency_is_bounded() {
        let c = local();
        c.set_monitor(true);
        for _ in 0..100 {
            c.push_capture(&tone(FRAME_SAMPLES, 300.0));
        }
        assert!(c.shared.mixer.lock().unwrap().monitor.len() <= MONITOR_MAX);
    }

    #[test]
    fn device_changes_bump_generation_only_when_different() {
        let c = local();
        let g0 = c.shared.device_generation();
        c.set_devices("", "");
        assert_eq!(c.shared.device_generation(), g0);
        c.set_devices("USB Mic", "");
        assert_eq!(c.shared.device_generation(), g0 + 1);
    }

    #[test]
    fn per_player_volume_mutes_only_that_player() {
        let c = local();
        let (a, b) = ([1; 16], [2; 16]);
        c.set_player_volume(a, 0.0);
        assert_eq!(energy(&feed(&c, a, [0.0, 0.0, 2.0], false, 300.0, 20)), 0.0);
        assert!(energy(&feed(&c, b, [0.0, 0.0, 2.0], false, 300.0, 20)) > 10.0);
        c.set_player_volume(a, 1.0);
        assert!(c.shared.mixer.lock().unwrap().player_volume.is_empty(), "volver a 100 % no deja basura");
    }

    #[test]
    fn with_headphones_behind_sounds_different_from_in_front() {
        // Mirando al sur (yaw 0): un hablante delante y otro detrás a la misma
        // distancia. Con HRTF el de detrás pierde agudos; con paneo, solo volumen.
        let speaker = [6; 16];
        let treble = |hrtf: bool, z: f32| {
            let c = local();
            c.set_hrtf(hrtf);
            energy(&feed(&c, speaker, [0.0, 0.0, z], false, 6000.0, 30))
        };
        let bass = |hrtf: bool, z: f32| {
            let c = local();
            c.set_hrtf(hrtf);
            energy(&feed(&c, speaker, [0.0, 0.0, z], false, 300.0, 30))
        };
        let tilt = |hrtf: bool| (treble(hrtf, -3.0) / bass(hrtf, -3.0)) / (treble(hrtf, 3.0) / bass(hrtf, 3.0));
        let (with, without) = (tilt(true), tilt(false));
        println!("agudos/graves detrás ÷ delante: con 3D {with:.2}, sin 3D {without:.2}");
        assert!(with < 0.7, "con 3D lo de detrás suena más apagado");
        assert!((without - 1.0).abs() < 0.05, "el paneo simple no cambia el timbre");
    }

    #[test]
    fn looking_up_puts_overhead_voices_in_front() {
        // Hablante 4 bloques más arriba. Con pitch -90 queda de frente y debe sonar
        // igual que uno delante a la misma altura.
        let speaker = [7; 16];
        let heard = |rel: [f32; 3], pitch: f32| {
            let c = local();
            c.set_listener_pitch(pitch);
            feed(&c, speaker, rel, false, 6000.0, 30)
        };
        let ahead = energy(&heard([0.0, 0.0, 4.0], 0.0));
        let above_looking = energy(&heard([0.0, 4.0, 0.0], -90.0));
        let above_not = energy(&heard([0.0, 4.0, 0.0], 0.0));
        println!("6 kHz: delante {ahead:.1}, encima mirándole {above_looking:.1}, encima sin mirar {above_not:.1}");
        assert!((above_looking / ahead - 1.0).abs() < 0.1);
        assert!((above_not / ahead - 1.0).abs() > 0.1, "sin mirar arriba suena distinto");
    }

    #[test]
    fn through_an_open_door_the_voice_comes_from_the_door() {
        // Mirando al sur: hablante al este (izquierda) tras una pared y puerta abierta
        // al oeste (derecha); el camino mide 9 bloques.
        let speaker = [8; 16];
        let listen = |door: bool| {
            let c = local();
            c.set_hrtf(false);
            c.set_occlusion(speaker, 1.0);
            if door {
                c.set_sound_path(speaker, Some(SoundPath { dir: [-1.0, 0.0, 0.0], length: 9.0, corners: 1 }));
            }
            let out = feed(&c, speaker, [5.0, 0.0, 0.0], false, 2500.0, 40);
            let l: f32 = out.chunks(2).map(|s| s[0] * s[0]).sum();
            let r: f32 = out.chunks(2).map(|s| s[1] * s[1]).sum();
            (l, r)
        };
        let (wl, wr) = listen(false);
        let (dl, dr) = listen(true);
        println!("2,5 kHz tras la pared: L {wl:.2} R {wr:.2} · con la puerta: L {dl:.2} R {dr:.2}");
        assert!(wl > wr, "sin puerta, a través de la pared: por la izquierda");
        assert!(dr > dl, "con la puerta abierta a la derecha, se oye por la derecha");
        assert!(dl + dr > (wl + wr) * 4.0, "y mucho más claro que a través del muro");
    }

    #[test]
    fn a_path_is_ignored_when_the_line_is_clear() {
        let speaker = [9; 16];
        let clear = energy(&feed(&local(), speaker, [5.0, 0.0, 0.0], false, 2500.0, 30));
        let c = local();
        c.set_sound_path(speaker, Some(SoundPath { dir: [-1.0, 0.0, 0.0], length: 9.0, corners: 1 }));
        let with_path = energy(&feed(&c, speaker, [5.0, 0.0, 0.0], false, 2500.0, 30));
        assert!((with_path / clear - 1.0).abs() < 0.01, "{with_path} vs {clear}");
    }

    #[test]
    fn a_nearby_wall_echoes_from_its_side() {
        // Hablante delante. Pared a 6 bloques al oeste (derecha mirando al sur): la
        // reflexión debe llegar por la derecha.
        let speaker = [10; 16];
        let side = |wall: bool| {
            let c = local();
            c.set_hrtf(false);
            let refl = if wall {
                vec![Reflection { dir: [-1.0, 0.0, 0.0], delay: 12.0 / 343.0, gain: 0.8 }]
            } else {
                vec![]
            };
            c.set_acoustics(0.4, 0.2, 0.3, refl);
            let out = feed(&c, speaker, [0.0, 0.0, 3.0], false, 400.0, 40);
            let l: f32 = out.chunks(2).map(|s| s[0] * s[0]).sum();
            let r: f32 = out.chunks(2).map(|s| s[1] * s[1]).sum();
            r / l
        };
        let (none, wall) = (side(false), side(true));
        println!("derecha/izquierda: sin pared {none:.2}, con pared a la derecha {wall:.2}");
        assert!(wall > none * 1.2);
    }

    /// Hueco de 3x3 de piedra: las cuatro paredes a la misma distancia. La suma de
    /// reflexiones no debe pasar de -6 dB respecto a la voz.
    #[test]
    fn a_tiny_stone_room_does_not_make_voices_metallic() {
        let speaker = [12; 16];
        // Reflexiones que mide Java en un 3x3: ocho fuertes casi a la vez.
        let box_reflections: Vec<Reflection> = [
            [0.22, 0.98, 0.0], [-0.61, 0.79, -0.11], [-0.19, 0.69, 0.70], [0.75, 0.60, 0.28],
            [0.37, 0.50, -0.78], [-0.99, 0.17, 0.04], [-0.05, 0.07, 1.0], [-0.64, 0.02, -0.77],
        ]
        .iter()
        .enumerate()
        .map(|(i, d)| Reflection { dir: *d, delay: if i == 0 { 0.0087 } else { 0.0117 }, gain: 0.63 })
        .collect();
        let play = |acoustics: bool| {
            let c = local();
            c.set_hrtf(false);
            if acoustics {
                c.set_acoustics(0.45, 0.14, 0.21, box_reflections.clone());
            }
            let out = feed(&c, speaker, [1.0, 0.0, 0.5], false, 300.0, 40);
            (energy(&out), out.iter().fold(0f32, |m, s| m.max(s.abs())))
        };
        let (dry, _) = play(false);
        let (room, peak) = play(true);
        let db = 10.0 * (room / dry).log10();
        println!("3×3 de piedra: {db:+.1} dB sobre la voz sola, pico {peak:.2}");
        assert!(db < 4.0, "los ecos tapan la voz: {db:+.1} dB");
        assert!(peak < 0.9, "satura: {peak}");
    }

    #[test]
    fn you_hear_your_own_echo_only_in_big_places() {
        let own_echo = |room: f32| {
            let c = local();
            c.set_noise_suppression(false);
            c.set_mode(TransmitMode::PushToTalk);
            c.set_push_to_talk(true);
            c.set_acoustics(0.45, room, 0.3, vec![]);
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            let mut e = 0f32;
            for n in 0..40 {
                c.push_capture(&tone(FRAME_SAMPLES, 300.0 + n as f32));
                c.render(&mut out);
                e += energy(&out);
            }
            e
        };
        let (tiny, cave) = (own_echo(0.14), own_echo(0.8));
        println!("tu eco: cuarto pequeño {tiny:.2}, cueva grande {cave:.2}");
        assert!(tiny < 0.01, "en un hueco pequeño no te oyes doble");
        assert!(cave > 1.0, "en una cueva grande sí oyes tu eco");
    }

    #[test]
    fn walls_muffle_volume_and_treble() {
        let speaker = [5; 16];
        // A 3 kHz la pared debe quitar mucha más energía que a un grave.
        let open = energy(&feed(&local(), speaker, [0.0, 0.0, 3.0], false, 3000.0, 30));
        let c = local();
        c.set_occlusion(speaker, 1.0);
        let walled = energy(&feed(&c, speaker, [0.0, 0.0, 3.0], false, 3000.0, 30));
        println!("agudos: abierto {open:.1}, tras pared {walled:.1}");
        assert!(walled < open * 0.05, "una pared apaga los agudos: {walled} vs {open}");

        let open_low = energy(&feed(&local(), speaker, [0.0, 0.0, 3.0], false, 200.0, 30));
        let c = local();
        c.set_occlusion(speaker, 1.0);
        let walled_low = energy(&feed(&c, speaker, [0.0, 0.0, 3.0], false, 200.0, 30));
        assert!(walled_low > open_low * 0.03, "los graves atraviesan algo: {walled_low} vs {open_low}");
        assert!(walled_low / open_low > walled / open, "la pared filtra más los agudos que los graves");
    }

    #[test]
    fn group_voice_ignores_distance_and_is_centered() {
        let c = local();
        // A 5000 bloques: fuera de alcance por proximidad, pero se oye por grupo.
        let out = feed(&c, [3; 16], [5000.0, 0.0, 0.0], true, 300.0, 20);
        assert!(energy(&out) > 10.0);
        assert!(out.chunks(2).all(|lr| (lr[0] - lr[1]).abs() < 1e-6), "centrada");
        assert!(c.speaking_players().iter().any(|(_, group)| *group));
    }

    #[test]
    fn cave_echo_adds_a_tail() {
        let speaker = [4; 16];
        let render_after = |wet: f32| {
            let c = local();
            c.set_environment(wet, 1.0);
            feed(&c, speaker, [0.0, 0.0, 2.0], false, 300.0, 15);
            // ¿Sigue sonando algo después de que deje de hablar?
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            let mut tail = 0.0;
            for i in 0..30 {
                c.render(&mut out);
                if i >= 12 {
                    tail += energy(&out);
                }
            }
            tail
        };
        let dry = render_after(0.0);
        let cave = render_after(0.6);
        println!("cola: seco {dry:.4}, cueva {cave:.4}");
        assert!(dry < 1e-6);
        assert!(cave > 0.01, "en una cueva la voz deja eco");
    }

    #[test]
    fn bitrate_policy() {
        assert_eq!(bitrate_for_loss(0), 32_000);
        assert_eq!(bitrate_for_loss(5), 24_000);
        assert_eq!(bitrate_for_loss(10), 18_000);
        assert_eq!(bitrate_for_loss(40), 14_000);
    }
}
