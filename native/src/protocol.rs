//! Protocolo de red de Ferrovoz (UDP o túnel por la conexión del juego).
//!
//! Todos los paquetes empiezan con una cabecera de 4 bytes en claro:
//!
//! ```text
//! +------+------+---------+------+
//! | 'F'  | 'V'  | version | tipo |
//! +------+------+---------+------+
//! ```
//!
//! El resto va cifrado con ChaCha20-Poly1305. Cada jugador tiene una clave de
//! 32 bytes que recibe por el canal del juego al entrar. La cabecera (y el UUID en
//! los paquetes de subida) va como AAD, así que no se puede cambiar el tipo ni
//! suplantar a otro jugador.
//!
//! Nonce: `dirección | 0 0 0 | contador` (64 bits). El receptor rechaza contadores
//! repetidos o demasiado viejos con una ventana de 64, como WireGuard. La dirección
//! evita que cliente y servidor reutilicen un nonce con la misma clave.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

pub const MAGIC: [u8; 2] = *b"FV";
/// v3: nonces con contador, GroupState, reverb y oclusión en el audio.
pub const VERSION: u8 = 3;

/// Frecuencia de muestreo de todo el pipeline.
pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms a 48 kHz, trama estándar de Opus para voz.
pub const FRAME_SAMPLES: usize = 960;
/// Tamaño máximo de datagrama.
pub const MAX_PACKET: usize = 1500;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const HEADER_LEN: usize = 4;
pub const UUID_LEN: usize = 16;

pub type PlayerKey = [u8; KEY_LEN];
pub type PlayerId = [u8; UUID_LEN];

/// Emisor del murmullo de la multitud (ver [`crate::crowd`]). No es un jugador.
pub const CROWD_ID: PlayerId = *b"ferrovoz:crowd!!";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    /// C -> S: registra la dirección del cliente (prueba que tiene la clave).
    Hello = 1,
    /// C -> S: trama Opus del micro.
    AudioUp = 2,
    /// S -> C: trama Opus de otro jugador con su posición relativa.
    AudioDown = 3,
    /// C -> S: keepalive del NAT y ping.
    Ping = 4,
    /// S -> C: respuesta al ping / confirmación del Hello.
    Pong = 5,
    /// S -> C: grupo actual.
    GroupState = 6,
    /// C -> S: el cliente deja la voz. No se le envía nada hasta el siguiente Hello.
    /// Servidores anteriores a 0.10 lo descartan.
    Bye = 7,
    /// Proxy -> C: voz de un grupo de red. Mismo cuerpo que `AudioDown`, con su
    /// propia dirección de nonce y ventana anti-replay para que proxy y relé nunca
    /// compartan nonce con la misma clave.
    RelayAudio = 8,
    /// Proxy -> C: grupo de red y nombres de sus miembros.
    RelayGroup = 9,
    /// C -> S: micro silenciado / ensordecido. Servidores anteriores lo descartan.
    VoiceState = 10,
    /// S -> C: estado de voz de los jugadores (ver [`PeerStates`]).
    PeerStates = 11,
}

impl PacketType {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Hello,
            2 => Self::AudioUp,
            3 => Self::AudioDown,
            4 => Self::Ping,
            5 => Self::Pong,
            6 => Self::GroupState,
            7 => Self::Bye,
            8 => Self::RelayAudio,
            9 => Self::RelayGroup,
            10 => Self::VoiceState,
            11 => Self::PeerStates,
            _ => return None,
        })
    }

    /// Paquetes con el UUID del remitente en claro tras la cabecera.
    fn carries_sender_id(self) -> bool {
        matches!(self, Self::Hello | Self::AudioUp | Self::Ping | Self::Bye | Self::VoiceState)
    }

    /// Dirección del tipo de paquete.
    pub fn direction(self) -> Direction {
        if self.carries_sender_id() {
            Direction::Up
        } else if matches!(self, Self::RelayAudio | Self::RelayGroup) {
            Direction::Relay
        } else {
            Direction::Down
        }
    }
}

/// Dirección del paquete; forma parte del nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Direction {
    /// Cliente -> servidor.
    Up = 0,
    /// Servidor -> cliente.
    Down = 1,
    /// Proxy -> cliente.
    Relay = 2,
}

#[derive(Debug, PartialEq)]
pub enum ProtocolError {
    TooShort,
    BadMagic,
    BadVersion(u8),
    UnknownType(u8),
    Crypto,
    Malformed,
    Replay,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ProtocolError {}

// ---------------------------------------------------------------------------
// Mensajes descifrados
// ---------------------------------------------------------------------------

/// Alcance de la voz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TalkMode {
    #[default]
    Normal = 0,
    Whisper = 1,
    Shout = 2,
}

impl TalkMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Whisper,
            2 => Self::Shout,
            _ => Self::Normal,
        }
    }
}

/// Reverb del sitio del hablante, cuantizada a 0..=255.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Environment {
    pub wet: u8,
    pub room: u8,
}

impl Environment {
    pub fn from_f32(wet: f32, room: f32) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self { wet: q(wet), room: q(room) }
    }
    pub fn wet(self) -> f32 {
        self.wet as f32 / 255.0
    }
    pub fn room(self) -> f32 {
        self.room as f32 / 255.0
    }
}

/// Trama subida por un cliente.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioUp {
    pub seq: u32,
    pub talk: TalkMode,
    pub env: Environment,
    pub opus: Vec<u8>,
}

/// Oclusión no calculada.
pub const OCCLUSION_UNKNOWN: u8 = 255;

/// Trama reenviada por el servidor a un oyente.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioDown {
    pub sender: PlayerId,
    pub seq: u32,
    /// Posición del hablante relativa al oyente (bloques). Al ser relativa cabe en
    /// f32 sin perder precisión aunque las coordenadas sean enormes.
    pub rel: [f32; 3],
    /// Alcance con el que habló; el oyente atenúa hasta esta distancia.
    pub range: f32,
    /// Voz de grupo: sin posición ni distancia.
    pub group: bool,
    /// El hablante tiene la cabeza bajo el agua. Va en un bit libre de los flags
    /// (los clientes antiguos lo ignoran).
    pub underwater: bool,
    /// Reverb del sitio del hablante.
    pub env: Environment,
    /// Oclusión calculada por el servidor (0..=254) o [`OCCLUSION_UNKNOWN`]. El
    /// cliente la usa cuando no puede calcularla (hablante fuera de su render distance).
    pub occlusion: u8,
    pub opus: Vec<u8>,
}

/// Cuerpo del Pong: pérdida que ve el servidor en la subida.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PongInfo {
    /// 0..=100
    pub loss_percent: u8,
    /// El servidor ha desactivado el eco de cuevas y salas. Va en un segundo byte
    /// que los clientes anteriores a 0.11 no leen.
    pub no_cave_echo: bool,
}

/// Grupo actual (nombre vacío = ninguno).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupState {
    pub name: String,
    pub owner: bool,
    pub persistent: bool,
}

/// Grupo de red: nombre, si el receptor es el dueño y los miembros con su nombre
/// (los de otros servidores no aparecen en la lista de jugadores).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetGroupState {
    pub name: String,
    pub owner: bool,
    pub members: Vec<(PlayerId, String)>,
}

impl NetGroupState {
    const OWNER: u8 = 1;
    /// Miembros que caben en un paquete (nombres de hasta 16 caracteres).
    pub const MAX_MEMBERS: usize = 32;

    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![if self.owner { Self::OWNER } else { 0 }];
        push_str(&mut v, &self.name);
        let members = &self.members[..self.members.len().min(Self::MAX_MEMBERS)];
        v.push(members.len() as u8);
        for (id, name) in members {
            v.extend_from_slice(id);
            push_str(&mut v, name);
        }
        v
    }

    pub fn decode(b: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader(b);
        let flags = r.u8()?;
        let name = r.str()?;
        let n = r.u8()? as usize;
        let mut members = Vec::with_capacity(n);
        for _ in 0..n {
            let mut id = [0u8; UUID_LEN];
            id.copy_from_slice(r.take(UUID_LEN)?);
            members.push((id, r.str()?));
        }
        Ok(Self { name, owner: flags & Self::OWNER != 0, members })
    }
}

/// Estado de voz de los jugadores. Quien no aparece no tiene el mod (o es incompatible).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PeerStates {
    pub entries: Vec<(PlayerId, u8)>,
}

impl PeerStates {
    /// No se le oye: mute propio, de un admin o sin permiso.
    pub const MUTED: u8 = 1;
    /// No oye a nadie.
    pub const DEAFENED: u8 = 2;
    /// Voz conectada. Si no: la ha desactivado o está conectando.
    pub const CONNECTED: u8 = 4;
    /// Entradas por paquete.
    pub const PER_PACKET: usize = 64;

    pub fn encode(&self) -> Vec<u8> {
        let entries = &self.entries[..self.entries.len().min(Self::PER_PACKET)];
        let mut v = Vec::with_capacity(1 + entries.len() * (UUID_LEN + 1));
        v.push(entries.len() as u8);
        for (id, flags) in entries {
            v.extend_from_slice(id);
            v.push(*flags);
        }
        v
    }

    pub fn decode(b: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader(b);
        let n = r.u8()? as usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let mut id = [0u8; UUID_LEN];
            id.copy_from_slice(r.take(UUID_LEN)?);
            entries.push((id, r.u8()?));
        }
        Ok(Self { entries })
    }
}

/// Texto con 1 byte de longitud delante (máx. 255 bytes, sin cortar caracteres).
fn push_str(v: &mut Vec<u8>, s: &str) {
    let mut end = s.len().min(255);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    v.push(end as u8);
    v.extend_from_slice(&s.as_bytes()[..end]);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtocolError> {
        if self.0.len() < n {
            return Err(ProtocolError::Malformed);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }

    fn str(&mut self) -> Result<String, ProtocolError> {
        let n = self.u8()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| ProtocolError::Malformed)
    }
}

// ---------------------------------------------------------------------------
// Cifrado y anti-replay
// ---------------------------------------------------------------------------

pub fn random_key() -> PlayerKey {
    let mut k = [0u8; KEY_LEN];
    getrandom::getrandom(&mut k).expect("no OS randomness source");
    k
}

fn nonce_for(direction: Direction, counter: u64) -> [u8; NONCE_LEN] {
    let mut n = [0u8; NONCE_LEN];
    n[0] = direction as u8;
    n[4..].copy_from_slice(&counter.to_be_bytes());
    n
}

fn header(t: PacketType) -> [u8; HEADER_LEN] {
    [MAGIC[0], MAGIC[1], VERSION, t as u8]
}

/// Construye un paquete: cabecera | `[uuid]` | nonce | cifrado(plaintext).
/// `counter` debe ser estrictamente creciente por clave y sentido.
pub fn seal(key: &PlayerKey, t: PacketType, sender: Option<&PlayerId>, plaintext: &[u8], counter: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + UUID_LEN + NONCE_LEN + plaintext.len() + TAG_LEN);
    out.extend_from_slice(&header(t));
    if t.carries_sender_id() {
        out.extend_from_slice(sender.expect("this packet type needs a sender"));
    }
    let aad_len = out.len();
    let nonce = nonce_for(t.direction(), counter);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &out[..aad_len] })
        .expect("ChaCha20-Poly1305 cannot fail with valid sizes");
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Ventana anti-replay de 64 paquetes.
#[derive(Debug, Default, Clone)]
pub struct ReplayWindow {
    highest: u64,
    /// Bit i = visto `highest - i`.
    seen: u64,
    any: bool,
}

impl ReplayWindow {
    const SIZE: u64 = 64;

    /// ¿Se aceptaría este contador? No lo marca.
    pub fn check(&self, counter: u64) -> bool {
        if !self.any || counter > self.highest {
            return true;
        }
        let age = self.highest - counter;
        age < Self::SIZE && self.seen & (1 << age) == 0
    }

    /// Marca el contador. Solo después de autenticar.
    pub fn mark(&mut self, counter: u64) {
        if !self.any {
            self.any = true;
            self.highest = counter;
            self.seen = 1;
        } else if counter > self.highest {
            let shift = counter - self.highest;
            self.seen = if shift >= Self::SIZE { 0 } else { self.seen << shift };
            self.seen |= 1;
            self.highest = counter;
        } else {
            self.seen |= 1 << (self.highest - counter);
        }
    }
}

/// Paquete recibido antes de descifrar (para buscar la clave del remitente).
#[derive(Debug)]
pub struct RawPacket<'a> {
    pub kind: PacketType,
    pub sender: Option<PlayerId>,
    aad: &'a [u8],
    nonce: &'a [u8],
    ciphertext: &'a [u8],
}

pub fn parse(buf: &[u8]) -> Result<RawPacket<'_>, ProtocolError> {
    if buf.len() < HEADER_LEN {
        return Err(ProtocolError::TooShort);
    }
    if buf[0..2] != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    if buf[2] != VERSION {
        return Err(ProtocolError::BadVersion(buf[2]));
    }
    let kind = PacketType::from_u8(buf[3]).ok_or(ProtocolError::UnknownType(buf[3]))?;
    let mut pos = HEADER_LEN;
    let sender = if kind.carries_sender_id() {
        if buf.len() < pos + UUID_LEN {
            return Err(ProtocolError::TooShort);
        }
        let mut id = [0u8; UUID_LEN];
        id.copy_from_slice(&buf[pos..pos + UUID_LEN]);
        pos += UUID_LEN;
        Some(id)
    } else {
        None
    };
    if buf.len() < pos + NONCE_LEN + TAG_LEN {
        return Err(ProtocolError::TooShort);
    }
    Ok(RawPacket {
        kind,
        sender,
        aad: &buf[..pos],
        nonce: &buf[pos..pos + NONCE_LEN],
        ciphertext: &buf[pos + NONCE_LEN..],
    })
}

impl RawPacket<'_> {
    /// Contador del nonce; error si la dirección no corresponde al tipo.
    pub fn counter(&self) -> Result<u64, ProtocolError> {
        if self.nonce[0] != self.kind.direction() as u8 || self.nonce[1..4] != [0, 0, 0] {
            return Err(ProtocolError::Malformed);
        }
        Ok(u64::from_be_bytes(self.nonce[4..].try_into().unwrap()))
    }

    pub fn open(&self, key: &PlayerKey) -> Result<Vec<u8>, ProtocolError> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
        cipher
            .decrypt(Nonce::from_slice(self.nonce), Payload { msg: self.ciphertext, aad: self.aad })
            .map_err(|_| ProtocolError::Crypto)
    }

    /// Comprueba la ventana, descifra y solo si es auténtico marca el contador.
    pub fn open_once(&self, key: &PlayerKey, window: &mut ReplayWindow) -> Result<Vec<u8>, ProtocolError> {
        let counter = self.counter()?;
        if !window.check(counter) {
            return Err(ProtocolError::Replay);
        }
        let body = self.open(key)?;
        window.mark(counter);
        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// Serialización (little endian)
// ---------------------------------------------------------------------------

impl AudioUp {
    const FIXED: usize = 4 + 1 + 2;

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::FIXED + self.opus.len());
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.push(self.talk as u8);
        v.push(self.env.wet);
        v.push(self.env.room);
        v.extend_from_slice(&self.opus);
        v
    }

    pub fn decode(b: &[u8]) -> Result<Self, ProtocolError> {
        if b.len() < Self::FIXED {
            return Err(ProtocolError::Malformed);
        }
        Ok(Self {
            seq: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            talk: TalkMode::from_u8(b[4]),
            env: Environment { wet: b[5], room: b[6] },
            opus: b[Self::FIXED..].to_vec(),
        })
    }
}

impl AudioDown {
    // uuid + seq + rel(3 x f32) + range(f32) + flags + wet + room + occlusion
    const FIXED: usize = UUID_LEN + 4 + 12 + 4 + 1 + 3;
    const FLAG_GROUP: u8 = 1;
    const FLAG_UNDERWATER: u8 = 2;

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::FIXED + self.opus.len());
        v.extend_from_slice(&self.sender);
        v.extend_from_slice(&self.seq.to_le_bytes());
        for c in self.rel {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.extend_from_slice(&self.range.to_le_bytes());
        let mut flags = 0;
        if self.group {
            flags |= Self::FLAG_GROUP;
        }
        if self.underwater {
            flags |= Self::FLAG_UNDERWATER;
        }
        v.push(flags);
        v.push(self.env.wet);
        v.push(self.env.room);
        v.push(self.occlusion);
        v.extend_from_slice(&self.opus);
        v
    }

    /// La misma trama para otro oyente: copia `template` (de [`encode`](Self::encode))
    /// cambiando solo lo que depende del oyente. Evita volver a montar la trama y
    /// copiar el Opus por separado para cada oyente.
    pub fn for_listener(template: &[u8], rel: [f32; 3], occlusion: u8) -> Vec<u8> {
        let mut v = template.to_vec();
        let at = UUID_LEN + 4;
        for (k, c) in rel.iter().enumerate() {
            v[at + 4 * k..at + 4 * k + 4].copy_from_slice(&c.to_le_bytes());
        }
        v[Self::FIXED - 1] = occlusion;
        v
    }

    pub fn decode(b: &[u8]) -> Result<Self, ProtocolError> {
        if b.len() < Self::FIXED {
            return Err(ProtocolError::Malformed);
        }
        let mut sender = [0u8; UUID_LEN];
        sender.copy_from_slice(&b[0..16]);
        let seq = u32::from_le_bytes(b[16..20].try_into().unwrap());
        let f = |i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let rel = [f(20), f(24), f(28)];
        let range = f(32);
        if !rel.iter().all(|c| c.is_finite()) || !range.is_finite() || range < 0.0 {
            return Err(ProtocolError::Malformed);
        }
        Ok(Self {
            sender,
            seq,
            rel,
            range,
            group: b[36] & Self::FLAG_GROUP != 0,
            underwater: b[36] & Self::FLAG_UNDERWATER != 0,
            env: Environment { wet: b[37], room: b[38] },
            occlusion: b[39],
            opus: b[Self::FIXED..].to_vec(),
        })
    }
}

impl PongInfo {
    const NO_CAVE_ECHO: u8 = 1;

    pub fn encode(&self) -> Vec<u8> {
        let flags = if self.no_cave_echo { Self::NO_CAVE_ECHO } else { 0 };
        vec![self.loss_percent.min(100), flags]
    }

    /// Un Pong de un servidor antiguo trae solo la pérdida.
    pub fn decode(b: &[u8]) -> Self {
        let flags = b.get(1).copied().unwrap_or(0);
        Self { loss_percent: b.first().copied().unwrap_or(0).min(100), no_cave_echo: flags & Self::NO_CAVE_ECHO != 0 }
    }
}

impl GroupState {
    const OWNER: u8 = 1;
    const PERSISTENT: u8 = 2;

    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![(if self.owner { Self::OWNER } else { 0 }) | (if self.persistent { Self::PERSISTENT } else { 0 })];
        v.extend_from_slice(self.name.as_bytes());
        v
    }

    pub fn decode(b: &[u8]) -> Result<Self, ProtocolError> {
        let (&flags, name) = b.split_first().ok_or(ProtocolError::Malformed)?;
        Ok(Self {
            name: String::from_utf8(name.to_vec()).map_err(|_| ProtocolError::Malformed)?,
            owner: flags & Self::OWNER != 0,
            persistent: flags & Self::PERSISTENT != 0,
        })
    }
}

/// Convierte el par (msb, lsb) que da `java.util.UUID` en 16 bytes.
pub fn uuid_from_java(msb: i64, lsb: i64) -> PlayerId {
    let mut id = [0u8; UUID_LEN];
    id[..8].copy_from_slice(&msb.to_be_bytes());
    id[8..].copy_from_slice(&lsb.to_be_bytes());
    id
}

pub fn uuid_to_java(id: &PlayerId) -> (i64, i64) {
    (
        i64::from_be_bytes(id[..8].try_into().unwrap()),
        i64::from_be_bytes(id[8..].try_into().unwrap()),
    )
}

/// UUID en formato de Java (`xxxxxxxx-xxxx-...`).
pub fn uuid_to_string(id: &PlayerId) -> String {
    let h: String = id.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn uuid_from_str(s: &str) -> Option<PlayerId> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut id = [0u8; UUID_LEN];
    for (i, b) in id.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_audio_up() {
        let key = random_key();
        let id = uuid_from_java(0x1234, -42);
        let msg = AudioUp { seq: 7, talk: TalkMode::Shout, env: Environment { wet: 10, room: 200 }, opus: vec![1, 2, 3, 4] };
        let pkt = seal(&key, PacketType::AudioUp, Some(&id), &msg.encode(), 5);
        let raw = parse(&pkt).unwrap();
        assert_eq!(raw.kind, PacketType::AudioUp);
        assert_eq!(raw.sender, Some(id));
        assert_eq!(raw.counter(), Ok(5));
        assert_eq!(AudioUp::decode(&raw.open(&key).unwrap()).unwrap(), msg);
    }

    #[test]
    fn roundtrip_audio_down() {
        let key = random_key();
        let msg = AudioDown {
            sender: [9; 16],
            seq: 99,
            rel: [1.5, -2.0, 30.25],
            range: 96.0,
            group: true,
            underwater: true,
            env: Environment::from_f32(0.5, 1.0),
            occlusion: 128,
            opus: vec![0xAA; 60],
        };
        let pkt = seal(&key, PacketType::AudioDown, None, &msg.encode(), 1);
        let raw = parse(&pkt).unwrap();
        assert_eq!(raw.sender, None);
        assert_eq!(AudioDown::decode(&raw.open(&key).unwrap()).unwrap(), msg);
    }

    #[test]
    fn group_state_roundtrip() {
        let g = GroupState { name: "Ñandúes".into(), owner: true, persistent: true };
        assert_eq!(GroupState::decode(&g.encode()).unwrap(), g);
        assert_eq!(GroupState::decode(&GroupState::default().encode()).unwrap(), GroupState::default());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let pkt = seal(&random_key(), PacketType::Ping, Some(&[1; 16]), &[], 0);
        assert_eq!(parse(&pkt).unwrap().open(&random_key()), Err(ProtocolError::Crypto));
    }

    #[test]
    fn spoofed_sender_is_rejected() {
        let key = random_key();
        let mut pkt = seal(&key, PacketType::AudioUp, Some(&[1; 16]), &[0; 7], 0);
        pkt[HEADER_LEN] ^= 0xFF;
        assert_eq!(parse(&pkt).unwrap().open(&key), Err(ProtocolError::Crypto));
    }

    #[test]
    fn replayed_packet_is_rejected() {
        let key = random_key();
        let mut window = ReplayWindow::default();
        let pkt = seal(&key, PacketType::Ping, Some(&[1; 16]), &[], 10);
        assert!(parse(&pkt).unwrap().open_once(&key, &mut window).is_ok());
        assert_eq!(parse(&pkt).unwrap().open_once(&key, &mut window), Err(ProtocolError::Replay));
    }

    #[test]
    fn forged_packet_does_not_advance_the_window() {
        let key = random_key();
        let mut window = ReplayWindow::default();
        let forged = seal(&random_key(), PacketType::Ping, Some(&[1; 16]), &[], 1_000_000);
        assert_eq!(parse(&forged).unwrap().open_once(&key, &mut window), Err(ProtocolError::Crypto));
        // Un contador bajo legítimo se sigue aceptando.
        let real = seal(&key, PacketType::Ping, Some(&[1; 16]), &[], 3);
        assert!(parse(&real).unwrap().open_once(&key, &mut window).is_ok());
    }

    #[test]
    fn replay_window_accepts_reordering_but_not_old_or_duplicates() {
        let mut w = ReplayWindow::default();
        for c in [5, 7, 6, 10, 8] {
            assert!(w.check(c), "{c}");
            w.mark(c);
        }
        assert!(!w.check(7), "duplicado");
        assert!(w.check(9), "hueco dentro de la ventana");
        w.mark(200);
        assert!(!w.check(100), "demasiado viejo");
        assert!(w.check(150), "dentro de la ventana de 64");
    }

    #[test]
    fn direction_mismatch_is_rejected() {
        // Paquete de bajada con el bit de dirección cambiado.
        let key = random_key();
        let mut pkt = seal(&key, PacketType::Pong, None, &[0], 1);
        pkt[HEADER_LEN] = Direction::Up as u8;
        assert_eq!(parse(&pkt).unwrap().counter(), Err(ProtocolError::Malformed));
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(parse(b"hola").unwrap_err(), ProtocolError::BadMagic);
        assert_eq!(parse(&[b'F', b'V', 9, 1]).unwrap_err(), ProtocolError::BadVersion(9));
        assert_eq!(parse(&[b'F', b'V', VERSION, 77]).unwrap_err(), ProtocolError::UnknownType(77));
        assert_eq!(parse(&[b'F', b'V', VERSION, 2, 0]).unwrap_err(), ProtocolError::TooShort);
    }

    #[test]
    fn pong_info_roundtrip_and_clamp() {
        let p = PongInfo { loss_percent: 12, no_cave_echo: true };
        assert_eq!(PongInfo::decode(&p.encode()), p);
        // Pong de un servidor antiguo: solo la pérdida, eco permitido.
        assert!(!PongInfo::decode(&[12]).no_cave_echo);
        assert_eq!(PongInfo::decode(&[250]).loss_percent, 100);
        assert_eq!(PongInfo::decode(&[]).loss_percent, 0);
    }

    #[test]
    fn unknown_talk_mode_is_normal() {
        assert_eq!(TalkMode::from_u8(77), TalkMode::Normal);
    }

    #[test]
    fn uuid_conversions() {
        let (m, l) = (i64::MIN + 5, -1);
        let id = uuid_from_java(m, l);
        assert_eq!(uuid_to_java(&id), (m, l));
        assert_eq!(uuid_from_str(&uuid_to_string(&id)), Some(id));
        assert_eq!(uuid_to_string(&uuid_from_java(0x0123_4567_89ab_cdef, 0)), "01234567-89ab-cdef-0000-000000000000");
    }

    #[test]
    fn audio_down_for_another_listener() {
        let base = AudioDown {
            sender: [7; 16],
            seq: 42,
            rel: [0.0; 3],
            range: 48.0,
            group: false,
            underwater: true,
            env: Environment { wet: 3, room: 4 },
            occlusion: 0,
            opus: vec![1, 2, 3, 4, 5],
        };
        let template = base.encode();
        let other = AudioDown { rel: [1.5, -2.0, 30.25], occlusion: 200, ..base.clone() };
        assert_eq!(AudioDown::for_listener(&template, other.rel, other.occlusion), other.encode());
    }

}
