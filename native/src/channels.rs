//! Canales de audio del servidor: sonido que no viene del micro de un jugador sino de
//! un plugin (la API de Java). El relé lo envía como si fuera alguien hablando, con
//! un emisor inventado, así que lo reproducen también los clientes que no saben nada
//! de canales.
//!
//! Los ids de canal empiezan por [`VIRTUAL_PREFIX`], como el del murmullo de la
//! multitud ([`crate::protocol::CROWD_ID`]): los clientes desde 0.12 no los muestran
//! como jugadores.

use crate::codec::VoiceEncoder;
use crate::protocol::PlayerId;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Primeros bytes de cualquier emisor que no es un jugador.
pub const VIRTUAL_PREFIX: [u8; 8] = *b"ferrovoz";
/// Bitrate por defecto: de sobra para voz y aceptable para música.
pub const DEFAULT_BITRATE: i32 = 48_000;
/// Canales a la vez como mucho, para que un plugin con fugas no se coma la memoria.
pub const MAX_CHANNELS: usize = 1024;
/// Alcance máximo de un canal con posición.
pub const MAX_RANGE: f32 = 30_000.0;

/// ¿Es un emisor del servidor (canal o murmullo) y no un jugador?
pub fn is_virtual(id: &PlayerId) -> bool {
    id[..VIRTUAL_PREFIX.len()] == VIRTUAL_PREFIX
}

/// Id nuevo para un canal: el prefijo más 8 bytes al azar.
pub fn new_id() -> PlayerId {
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&VIRTUAL_PREFIX);
    getrandom::getrandom(&mut id[8..]).expect("no OS randomness source");
    id
}

/// A quién le llega un canal.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Desde un punto del mundo, a quien esté dentro del alcance.
    Point { pos: [f64; 3], dim: i32 },
    /// Desde donde esté un jugador en cada trama (también lo oye él).
    Follow(PlayerId),
    /// Sin posición, solo a estos jugadores.
    Listeners(HashSet<PlayerId>),
    /// Sin posición, a los miembros de un grupo (los de ese momento).
    Group(String),
}

impl Target {
    /// Con posición (3D, distancia) o directo (como la voz de grupo).
    pub fn positional(&self) -> bool {
        matches!(self, Self::Point { .. } | Self::Follow(_))
    }
}

pub struct Channel {
    pub target: Target,
    pub range: f32,
    pub seq: u32,
    /// Aparte del estado del relé: codificar no debe bloquear la voz de los demás.
    pub encoder: Arc<Mutex<VoiceEncoder>>,
}

impl Channel {
    pub fn new(target: Target, range: f32, bitrate: i32, music: bool) -> Result<Self, opus::Error> {
        let mut enc = VoiceEncoder::with_application(bitrate.clamp(6_000, 256_000), music)?;
        enc.set_complexity(5);
        Ok(Self { target, range: clamp_range(range), seq: 0, encoder: Arc::new(Mutex::new(enc)) })
    }
}

pub fn clamp_range(range: f32) -> f32 {
    if range.is_finite() { range.clamp(1.0, MAX_RANGE) } else { MAX_RANGE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CROWD_ID;

    #[test]
    fn channel_ids_are_virtual_and_unique() {
        let a = new_id();
        let b = new_id();
        assert!(is_virtual(&a) && is_virtual(&b));
        assert_ne!(a, b);
        assert!(is_virtual(&CROWD_ID), "el murmullo tampoco es un jugador");
        assert!(!is_virtual(&[7u8; 16]));
    }

    #[test]
    fn ranges_are_kept_sane() {
        assert_eq!(clamp_range(0.0), 1.0);
        assert_eq!(clamp_range(f32::NAN), MAX_RANGE);
        assert_eq!(clamp_range(1e9), MAX_RANGE);
        assert_eq!(clamp_range(48.0), 48.0);
    }
}
