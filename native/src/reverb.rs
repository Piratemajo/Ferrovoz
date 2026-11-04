//! Reverb ligera para cuevas y salas.
//!
//! Schroeder/Freeverb: 4 peines en paralelo con amortiguación de agudos en la
//! realimentación y 2 paso todo. Entrada mono; la salida tiene un pequeño desfase
//! entre canales.
//!
//! Los parámetros vienen de Java (rayos): `wet`, `room` (RT60 de 0.25 a 3.5 s, por
//! Sabine según tamaño y materiales) y `damp` (absorción de agudos).
//!
//! [`EarlyReflections`]: una reflexión por pared cercana, con su retardo (ida y
//! vuelta a 343 m/s) y su dirección.

use crate::protocol::SAMPLE_RATE;

// Retardos de Freeverb (44.1 kHz) reescalados a 48 kHz.
const COMB_TUNING: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASS_TUNING: [usize; 2] = [556, 441];
const STEREO_SPREAD: usize = 23;

fn scale(samples: usize) -> usize {
    samples * SAMPLE_RATE as usize / 44_100
}

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len], pos: 0, store: 0.0 }
    }

    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = x + self.store * feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len], pos: 0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        let out = b - x;
        self.buf[self.pos] = x + b * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

struct Channel {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl Channel {
    fn new(spread: usize) -> Self {
        Self {
            combs: COMB_TUNING.iter().map(|&t| Comb::new(scale(t + spread))).collect(),
            allpasses: ALLPASS_TUNING.iter().map(|&t| Allpass::new(scale(t + spread))).collect(),
        }
    }

    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let mut acc = 0.0;
        for c in &mut self.combs {
            acc += c.process(x, feedback, damp);
        }
        for a in &mut self.allpasses {
            acc = a.process(acc);
        }
        acc
    }
}

pub struct Reverb {
    left: Channel,
    right: Channel,
    wet: f32,
    feedback: f32,
    damp: f32,
}

/// RT60 (s) para un `room` 0..1.
pub fn rt60_for_room(room: f32) -> f32 {
    0.25 + 3.25 * room.clamp(0.0, 1.0)
}

/// Inversa de [`rt60_for_room`].
pub fn room_for_rt60(rt60: f32) -> f32 {
    ((rt60 - 0.25) / 3.25).clamp(0.0, 1.0)
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Reverb {
    pub fn new() -> Self {
        Self { left: Channel::new(0), right: Channel::new(STEREO_SPREAD), wet: 0.0, feedback: 0.7, damp: 0.35 }
    }

    /// `wet` 0..1, `room` 0..1 (ver [`rt60_for_room`]).
    pub fn set_params(&mut self, wet: f32, room: f32) {
        self.wet = wet.clamp(0.0, 1.0);
        // Realimentación para caer 60 dB en RT60 (retardo medio de los peines ~28 ms).
        let comb_delay = scale(COMB_TUNING.iter().sum::<usize>() / COMB_TUNING.len()) as f32 / SAMPLE_RATE as f32;
        self.feedback = 10f32.powf(-3.0 * comb_delay / rt60_for_room(room)).min(0.96);
    }

    /// Absorción de agudos (0 = piedra, 1 = lana).
    pub fn set_damp(&mut self, damp: f32) {
        self.damp = damp.clamp(0.05, 0.9);
    }

    pub fn is_active(&self) -> bool {
        self.wet > 0.001
    }

    /// Procesa `input` (mono) y suma la reverb a `out` (estéreo intercalado).
    pub fn process_add(&mut self, input: &[f32], out: &mut [f32]) {
        let damp = self.damp;
        // Escalado de entrada como en Freeverb.
        let gain = 0.015 * self.wet * 4.0;
        for (i, &x) in input.iter().enumerate() {
            let x = x * gain;
            out[2 * i] += self.left.process(x, self.feedback, damp);
            out[2 * i + 1] += self.right.process(x, self.feedback, damp);
        }
    }
}

/// Reflexión temprana: una pared a cierta distancia y dirección.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reflection {
    /// Dirección hacia la pared (coordenadas del mundo, unitaria).
    pub dir: [f32; 3],
    /// Retardo respecto al directo (s).
    pub delay: f32,
    /// Ganancia (0..1), según material y distancia.
    pub gain: f32,
}

/// Máximo de reflexiones (Java elige las más fuertes y separadas).
pub const MAX_REFLECTIONS: usize = 8;
/// Retardo máximo: ida y vuelta a 32 bloques.
const ER_MAX_SECONDS: f32 = 0.2;

#[derive(Clone, Copy, Default)]
struct Tap {
    delay: f32,
    left: f32,
    right: f32,
}

/// Reflexiones tempranas: una línea de retardo con una salida por pared, paneada
/// según su posición respecto a la cabeza. Al cambiar, se interpola a lo largo de la
/// trama.
pub struct EarlyReflections {
    buf: Vec<f32>,
    pos: usize,
    taps: [Tap; MAX_REFLECTIONS],
    active: usize,
}

impl Default for EarlyReflections {
    fn default() -> Self {
        Self::new()
    }
}

impl EarlyReflections {
    pub fn new() -> Self {
        let len = (ER_MAX_SECONDS * SAMPLE_RATE as f32) as usize + 2;
        Self { buf: vec![0.0; len], pos: 0, taps: [Tap::default(); MAX_REFLECTIONS], active: 0 }
    }

    pub fn is_active(&self) -> bool {
        self.active > 0
    }

    /// `reflections`: (x derecha, z delante, retardo, ganancia) relativos a la cabeza.
    /// Procesa `input` (mono) y suma a `out` (estéreo intercalado).
    pub fn process_add(&mut self, reflections: &[(f32, f32, f32, f32)], input: &[f32], out: &mut [f32]) {
        let mut new = [Tap::default(); MAX_REFLECTIONS];
        let n_new = reflections.len().min(MAX_REFLECTIONS);
        for (t, &(x, z, delay, gain)) in new.iter_mut().zip(reflections) {
            // Paneo de potencia constante; lo de detrás algo más bajo.
            let angle = (x.clamp(-1.0, 1.0) * 0.85 + 1.0) * std::f32::consts::FRAC_PI_4;
            let behind = if z < 0.0 { 1.0 + 0.25 * z } else { 1.0 };
            *t = Tap {
                delay: (delay.clamp(0.0, ER_MAX_SECONDS) * SAMPLE_RATE as f32).max(1.0),
                left: angle.cos() * gain * behind,
                right: angle.sin() * gain * behind,
            };
        }
        let old = self.taps;
        let n_old = self.active;
        let len = self.buf.len();
        let n = input.len().min(out.len() / 2);
        for (i, &x) in input.iter().take(n).enumerate() {
            self.pos = (self.pos + 1) % len;
            self.buf[self.pos] = x;
            let f = (i + 1) as f32 / n as f32;
            let read = |d: f32| {
                let k = d.round() as usize;
                self.buf[(self.pos + len - k.min(len - 1)) % len]
            };
            let (mut l, mut r) = (0.0, 0.0);
            for t in &old[..n_old] {
                let s = read(t.delay) * (1.0 - f);
                l += s * t.left;
                r += s * t.right;
            }
            for t in &new[..n_new] {
                let s = read(t.delay) * f;
                l += s * t.left;
                r += s * t.right;
            }
            out[2 * i] += l;
            out[2 * i + 1] += r;
        }
        self.taps = new;
        self.active = n_new;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_on_the_right_echoes_on_the_right_after_the_round_trip() {
        // Pared a 10 bloques a la derecha: llega 2*10/343 = 58 ms después, por la derecha.
        let mut er = EarlyReflections::new();
        let delay = 20.0 / 343.0;
        let refl = [(1.0, 0.0, delay, 0.5)];
        let mut out = vec![0.0f32; 960 * 2 * 5];
        let mut input = vec![0.0f32; 960 * 5];
        input[0] = 1.0;
        for (inp, o) in input.chunks(960).zip(out.chunks_mut(1920)) {
            er.process_add(&refl, inp, o);
        }
        let peak = out.chunks(2).enumerate().max_by(|a, b| (a.1[1].abs()).partial_cmp(&b.1[1].abs()).unwrap()).unwrap();
        let expected = (delay * SAMPLE_RATE as f32).round() as usize;
        assert!((peak.0 as i64 - expected as i64).abs() <= 1, "eco en {} (esperado {expected})", peak.0);
        assert!(peak.1[1] > peak.1[0] * 3.0, "por la derecha");
    }

    #[test]
    fn rt60_sets_how_long_the_room_rings() {
        let tail = |room: f32| {
            let mut r = Reverb::new();
            r.set_params(0.5, room);
            let mut energy = 0.0;
            for block in 0..60 {
                let mut input = vec![0.0f32; 960];
                if block == 0 {
                    input[0] = 1.0;
                }
                let mut out = vec![0.0f32; 1920];
                r.process_add(&input, &mut out);
                if block >= 25 {
                    energy += out.iter().map(|s| s * s).sum::<f32>();
                }
            }
            energy
        };
        // A partir de 0.5 s: con RT60 0.25 s ya no suena; con 3.5 s, sí.
        assert!(tail(1.0) > tail(0.0) * 100.0);
        assert!((rt60_for_room(room_for_rt60(1.7)) - 1.7).abs() < 1e-4);
    }

    fn tail_energy(wet: f32, room: f32) -> f32 {
        let mut r = Reverb::new();
        r.set_params(wet, room);
        let n = 960;
        let mut impulse = vec![0.0f32; n];
        impulse[0] = 1.0;
        let mut energy = 0.0;
        // Impulso y 0.5 s de silencio para medir la cola.
        for block in 0..25 {
            let input = if block == 0 { impulse.clone() } else { vec![0.0; n] };
            let mut out = vec![0.0f32; n * 2];
            r.process_add(&input, &mut out);
            if block >= 5 {
                energy += out.iter().map(|s| s * s).sum::<f32>();
            }
        }
        energy
    }

    #[test]
    fn dry_room_has_no_tail() {
        assert_eq!(tail_energy(0.0, 0.5), 0.0);
    }

    #[test]
    fn cave_has_a_tail_and_bigger_rooms_last_longer() {
        let small = tail_energy(0.5, 0.0);
        let big = tail_energy(0.5, 1.0);
        assert!(small > 0.0);
        assert!(big > small * 2.0, "una caverna grande resuena más: {big} vs {small}");
    }

    #[test]
    fn stays_stable_with_max_settings() {
        let mut r = Reverb::new();
        r.set_params(1.0, 1.0);
        let input = vec![0.8f32; 960];
        let mut peak = 0.0f32;
        for _ in 0..200 {
            let mut out = vec![0.0f32; 1920];
            r.process_add(&input, &mut out);
            peak = out.iter().fold(peak, |m, s| m.max(s.abs()));
        }
        assert!(peak.is_finite() && peak < 4.0, "no debe explotar: {peak}");
    }
}
