//! Eventos para Java (avisos de grupo y API) y quién está hablando.

use super::*;

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
    /// Han borrado el grupo de `player` (uno por miembro, salvo quien lo borró).
    Dissolved,
    /// Primer Hello: ya le llega la voz.
    Connected,
    /// Ha dejado la voz (Bye o salió del servidor).
    Disconnected,
    /// Empieza a sonar su voz.
    StartedSpeaking,
    /// Lleva un momento (400 ms) sin sonar.
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
            Self::Dissolved => "dissolve",
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

impl VoiceServer {
    /// Eventos pendientes, en orden.
    pub fn poll_events(&self) -> Vec<ServerEvent> {
        self.state().events.drain(..).collect()
    }
}

impl State {
    pub(super) fn push_event(&mut self, kind: EventKind, detail: String, player: PlayerId) {
        if self.events.len() >= EVENTS_MAX {
            self.events.pop_front();
        }
        self.events.push_back(ServerEvent { kind, player, detail });
    }

    /// Eventos de un jugador que deja el relé (sale, o se le vuelve a registrar).
    pub(super) fn note_gone(&mut self, id: &PlayerId, p: &Player) {
        if p.speaking {
            self.push_event(EventKind::StoppedSpeaking, String::new(), *id);
        }
        if !matches!(p.transport, Transport::Pending) {
            self.push_event(EventKind::Disconnected, String::new(), *id);
        }
    }
}

impl Shared {
    /// Quien lleva un rato sin sonar deja de hablar.
    pub(super) fn sweep_speaking(&self) {
        let mut st = lock(&self.state);
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
}
