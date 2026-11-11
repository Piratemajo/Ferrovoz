//! Reverb ligera para cuevas y salas.
//!
//! Schroeder/Freeverb: 4 peines en paralelo con realimentación separada en graves y
//! agudos, y 2 paso todo. Delante lleva un pre-retardo. Entrada mono; la salida tiene
//! un pequeño desfase entre canales.
//!
//! Los parámetros vienen de Java (rayos): `wet`, `room` (RT60 de 0.25 a 3.5 s, por
//! Sabine según tamaño y materiales) y `damp` (absorción de agudos del material).
//!
//! Dos bandas porque una sala real no se apaga igual en todo el espectro: la piedra
//! devuelve los agudos y retumba largo en graves, mientras que la lana se los come
//! casi todos. Un solo mando no puede decir las dos cosas.
//!
//! El pre-retardo es lo que da tamaño: la cola difusa de una nave no empieza encima
//! de la voz, empieza cuando vuelven las primeras reflexiones.
//!
//! [`EarlyReflections`]: una reflexión por pared cercana, con su retardo (ida y
//! vuelta a 343 m/s), su dirección y el filtro del material que la devuelve.

use crate::protocol::SAMPLE_RATE;

// Retardos de Freeverb (44.1 kHz) reescalados a 48 kHz.
const COMB_TUNING: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASS_TUNING: [usize; 2] = [556, 441];
const STEREO_SPREAD: usize = 23;

/// Corte entre graves y agudos en la realimentación (Hz).
const CROSSOVER: f32 = 1_200.0;
/// Cuánto más duran los graves que la banda media medida, de piedra a lana.
const LOW_RATIO: [f32; 2] = [1.35, 1.1];
/// Cuánto menos duran los agudos, de piedra a lana.
const HIGH_RATIO: [f32; 2] = [0.8, 0.3];
/// Realimentación máxima: por encima la reverb se vuelve inestable.
const MAX_FEEDBACK: f32 = 0.96;
/// Pre-retardo máximo (s): ida y vuelta a 13 bloques.
const MAX_PRE_DELAY: f32 = 0.08;
/// Pre-retardo mínimo (s): un cuarto pequeño.
const MIN_PRE_DELAY: f32 = 0.005;
/// Paso alto de la entrada (Hz).
///
/// Ninguna sala reverbera a 0 Hz y la voz no tiene nada por debajo de 80, pero con
/// los graves durando casi 5 s la continua se acumularía en los peines hasta
/// saturar. Quitarla de la entrada también deja la cola más limpia.
const INPUT_HIGHPASS: f32 = 80.0;

fn scale(samples: usize) -> usize {
    samples * SAMPLE_RATE as usize / 44_100
}

/// Coeficiente de un paso bajo de un polo para un corte en Hz.
fn one_pole(cutoff: f32) -> f32 {
    (1.0 - (-2.0 * std::f32::consts::PI * cutoff / SAMPLE_RATE as f32).exp()).clamp(0.0, 1.0)
}

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    low: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len], pos: 0, low: 0.0 }
    }

    /// `cross` separa la señal realimentada en graves y agudos, cada uno con su
    /// propia realimentación: así los graves pueden durar más que los agudos.
    fn process(&mut self, x: f32, low_fb: f32, high_fb: f32, cross: f32) -> f32 {
        let out = self.buf[self.pos];
        self.low += cross * (out - self.low);
        let high = out - self.low;
        self.buf[self.pos] = x + self.low * low_fb + high * high_fb;
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

    fn process(&mut self, x: f32, low_fb: f32, high_fb: f32, cross: f32) -> f32 {
        let mut acc = 0.0;
        for c in &mut self.combs {
            acc += c.process(x, low_fb, high_fb, cross);
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
    low_fb: f32,
    high_fb: f32,
    cross: f32,
    /// Pre-retardo: línea de retardo delante de la reverb.
    pre: Vec<f32>,
    pre_pos: usize,
    pre_samples: usize,
    /// Estado del paso alto de la entrada.
    dc: f32,
    dc_cut: f32,
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
        let pre_len = (MAX_PRE_DELAY * SAMPLE_RATE as f32) as usize + 2;
        let mut r = Self {
            left: Channel::new(0),
            right: Channel::new(STEREO_SPREAD),
            wet: 0.0,
            low_fb: 0.7,
            high_fb: 0.5,
            cross: one_pole(CROSSOVER),
            pre: vec![0.0; pre_len],
            pre_pos: 0,
            pre_samples: (MIN_PRE_DELAY * SAMPLE_RATE as f32) as usize,
            dc: 0.0,
            dc_cut: one_pole(INPUT_HIGHPASS),
        };
        r.set_params(0.0, 0.5, 0.3);
        r
    }

    /// Retardo medio de los peines (s): la referencia para calcular la realimentación.
    fn comb_delay() -> f32 {
        scale(COMB_TUNING.iter().sum::<usize>() / COMB_TUNING.len()) as f32 / SAMPLE_RATE as f32
    }

    /// Realimentación para que una banda caiga 60 dB en `rt60` segundos.
    fn feedback_for(rt60: f32) -> f32 {
        10f32.powf(-3.0 * Self::comb_delay() / rt60.max(0.05)).min(MAX_FEEDBACK)
    }

    /// `wet` 0..1, `room` 0..1 (ver [`rt60_for_room`]) y `damp` 0..1 (0 = piedra,
    /// 1 = lana), que decide cuánto más duran los graves que los agudos.
    pub fn set_params(&mut self, wet: f32, room: f32, damp: f32) {
        self.wet = wet.clamp(0.0, 1.0);
        let d = damp.clamp(0.0, 1.0);
        let rt60 = rt60_for_room(room);
        let low = LOW_RATIO[0] + (LOW_RATIO[1] - LOW_RATIO[0]) * d;
        let high = HIGH_RATIO[0] + (HIGH_RATIO[1] - HIGH_RATIO[0]) * d;
        self.low_fb = Self::feedback_for(rt60 * low);
        self.high_fb = Self::feedback_for(rt60 * high);
    }

    /// Pre-retardo en segundos: cuánto tarda en empezar la cola difusa. Se saca del
    /// tiempo que tardan en volver las primeras reflexiones.
    pub fn set_pre_delay(&mut self, seconds: f32) {
        let s = seconds.clamp(MIN_PRE_DELAY, MAX_PRE_DELAY);
        self.pre_samples = ((s * SAMPLE_RATE as f32) as usize).max(1).min(self.pre.len() - 1);
    }

    /// Procesa `input` (mono) y suma la reverb a `out` (estéreo intercalado).
    pub fn process_add(&mut self, input: &[f32], out: &mut [f32]) {
        // Escalado de entrada como en Freeverb.
        let gain = 0.015 * self.wet * 4.0;
        let len = self.pre.len();
        let n = input.len().min(out.len() / 2);
        for (i, &x) in input.iter().take(n).enumerate() {
            self.dc += self.dc_cut * (x - self.dc);
            self.pre_pos = (self.pre_pos + 1) % len;
            self.pre[self.pre_pos] = x - self.dc;
            let x = self.pre[(self.pre_pos + len - self.pre_samples) % len] * gain;
            out[2 * i] += self.left.process(x, self.low_fb, self.high_fb, self.cross);
            out[2 * i + 1] += self.right.process(x, self.low_fb, self.high_fb, self.cross);
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
    /// Absorción del material que devuelve el sonido (0 = piedra, 1 = lana): la
    /// piedra devuelve un chasquido brillante y la lana un golpe sordo.
    pub damp: f32,
}

/// Máximo de reflexiones (Java elige las más fuertes y separadas).
pub const MAX_REFLECTIONS: usize = 8;
/// Retardo máximo: ida y vuelta a 32 bloques.
const ER_MAX_SECONDS: f32 = 0.2;
/// Corte del filtro de una pared que no absorbe nada (Hz).
const WALL_BRIGHT: f32 = 14_000.0;
/// Factor del corte con absorción 1: 14 kHz * 0.06 ~ 840 Hz.
const WALL_DULL: f32 = 0.06;

#[derive(Clone, Copy, Default, PartialEq)]
struct Tap {
    delay: f32,
    left: f32,
    right: f32,
    /// Coeficiente del paso bajo del material.
    cut: f32,
}

/// Reflexiones tempranas: una línea de retardo con una salida por pared, paneada
/// según su posición respecto a la cabeza y filtrada según el material. Al cambiar,
/// se interpola a lo largo de la trama.
pub struct EarlyReflections {
    buf: Vec<f32>,
    pos: usize,
    taps: [Tap; MAX_REFLECTIONS],
    /// Estado del paso bajo de cada salida; se queda entre tramas.
    lowpass: [f32; MAX_REFLECTIONS],
    /// Estado del filtro de las salidas que se apagan mientras se cambia de pared.
    fading: [f32; MAX_REFLECTIONS],
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
        Self {
            buf: vec![0.0; len],
            pos: 0,
            taps: [Tap::default(); MAX_REFLECTIONS],
            lowpass: [0.0; MAX_REFLECTIONS],
            fading: [0.0; MAX_REFLECTIONS],
            active: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active > 0
    }

    /// Retardo medio de las paredes (s), o `None` si no hay ninguna. Es cuándo
    /// empieza a construirse el campo difuso.
    pub fn mean_delay(reflections: &[(f32, f32, f32, f32, f32)]) -> Option<f32> {
        if reflections.is_empty() {
            return None;
        }
        Some(reflections.iter().map(|r| r.2).sum::<f32>() / reflections.len() as f32)
    }

    /// `reflections`: (x derecha, z delante, retardo, ganancia, absorción) relativos a
    /// la cabeza. Procesa `input` (mono) y suma a `out` (estéreo intercalado).
    pub fn process_add(&mut self, reflections: &[(f32, f32, f32, f32, f32)], input: &[f32], out: &mut [f32]) {
        let mut new = [Tap::default(); MAX_REFLECTIONS];
        let n_new = reflections.len().min(MAX_REFLECTIONS);
        for (t, &(x, z, delay, gain, damp)) in new.iter_mut().zip(reflections) {
            // Paneo de potencia constante; lo de detrás algo más bajo.
            let angle = (x.clamp(-1.0, 1.0) * 0.85 + 1.0) * std::f32::consts::FRAC_PI_4;
            let behind = if z < 0.0 { 1.0 + 0.25 * z } else { 1.0 };
            *t = Tap {
                delay: (delay.clamp(0.0, ER_MAX_SECONDS) * SAMPLE_RATE as f32).max(1.0),
                left: angle.cos() * gain * behind,
                right: angle.sin() * gain * behind,
                cut: one_pole(WALL_BRIGHT * WALL_DULL.powf(damp.clamp(0.0, 1.0))),
            };
        }
        // Mientras no cambien las paredes no hay nada que cruzar, que es el caso
        // normal: Java solo las recalcula unas pocas veces por segundo.
        let changed = self.active != n_new || self.taps[..n_new] != new[..n_new];
        if changed {
            // Las salidas que se apagan se llevan su filtro aparte, para que las
            // nuevas puedan tener otro material sin pisarlo.
            self.fading = self.lowpass;
        }
        let old = self.taps;
        let n_old = if changed { self.active } else { 0 };
        let len = self.buf.len();
        let n = input.len().min(out.len() / 2);
        for (i, &x) in input.iter().take(n).enumerate() {
            self.pos = (self.pos + 1) % len;
            self.buf[self.pos] = x;
            let f = if changed { (i + 1) as f32 / n as f32 } else { 1.0 };
            let read = |d: f32| {
                let k = d.round() as usize;
                self.buf[(self.pos + len - k.min(len - 1)) % len]
            };
            let (mut l, mut r) = (0.0, 0.0);
            for (s, t) in old[..n_old].iter().enumerate() {
                let x = read(t.delay);
                self.fading[s] += t.cut * (x - self.fading[s]);
                let v = self.fading[s] * (1.0 - f);
                l += v * t.left;
                r += v * t.right;
            }
            for (s, t) in new[..n_new].iter().enumerate() {
                let x = read(t.delay);
                self.lowpass[s] += t.cut * (x - self.lowpass[s]);
                let v = self.lowpass[s] * f;
                l += v * t.left;
                r += v * t.right;
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

    /// Energía por encima de `cutoff` dividida entre la energía total, con una DFT a mano.
    fn high_fraction(samples: &[f32], cutoff: f32) -> f32 {
        let n = samples.len();
        let bins = 48;
        let (mut high, mut total) = (0.0f32, 0.0f32);
        for b in 1..=bins {
            let freq = b as f32 * SAMPLE_RATE as f32 / (2.0 * bins as f32);
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in samples.iter().enumerate() {
                let w = 2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32;
                re += s * w.cos();
                im -= s * w.sin();
            }
            let e = (re * re + im * im) / n as f32;
            total += e;
            if freq > cutoff {
                high += e;
            }
        }
        if total <= 0.0 {
            0.0
        } else {
            high / total
        }
    }

    #[test]
    fn a_wall_on_the_right_echoes_on_the_right_after_the_round_trip() {
        // Pared a 10 bloques a la derecha: llega 2*10/343 = 58 ms después, por la derecha.
        let mut er = EarlyReflections::new();
        let delay = 20.0 / 343.0;
        let refl = [(1.0, 0.0, delay, 0.5, 0.0)];
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
    fn a_wool_wall_returns_a_duller_echo_than_stone() {
        let echo = |damp: f32| {
            let mut er = EarlyReflections::new();
            let refl = [(0.0, 1.0, 20.0 / 343.0, 1.0, damp)];
            let mut out = vec![0.0f32; 960 * 2 * 6];
            let mut input = vec![0.0f32; 960 * 6];
            // Ruido corto: tiene energía en todo el espectro.
            let mut seed = 12_345u32;
            for s in input.iter_mut().take(480) {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                *s = (seed >> 16) as f32 / 32_768.0 - 1.0;
            }
            for (inp, o) in input.chunks(960).zip(out.chunks_mut(1920)) {
                er.process_add(&refl, inp, o);
            }
            let mono: Vec<f32> = out.chunks(2).map(|c| c[0] + c[1]).collect();
            high_fraction(&mono, 4_000.0)
        };
        let stone = echo(0.03);
        let wool = echo(0.7);
        // Un polo cae 6 dB por octava: la mitad de agudos es todo lo que da de sí, y
        // es justo lo que se oye como "pared blanda".
        assert!(stone > wool * 1.8, "la piedra devuelve más agudos que la lana: {stone} vs {wool}");
    }

    #[test]
    fn the_tail_starts_after_the_pre_delay() {
        let mut r = Reverb::new();
        r.set_params(1.0, 0.8, 0.1);
        r.set_pre_delay(0.06);
        let mut input = vec![0.0f32; 960 * 8];
        input[0] = 1.0;
        let mut out = vec![0.0f32; 960 * 2 * 8];
        for (inp, o) in input.chunks(960).zip(out.chunks_mut(1920)) {
            r.process_add(inp, o);
        }
        let first = out.chunks(2).position(|c| c[0].abs() > 1e-5).expect("debe sonar");
        let expected = (0.06 * SAMPLE_RATE as f32) as usize;
        // La cola empieza tras el pre-retardo, no encima de la voz.
        assert!(first >= expected, "la cola empieza en {first}, antes del pre-retardo {expected}");
        assert!(first < expected + 2_000, "pero no mucho después: {first}");
    }

    #[test]
    fn stone_rings_longer_in_the_bass_than_wool() {
        let tail = |damp: f32| {
            let mut r = Reverb::new();
            r.set_params(1.0, 0.7, damp);
            r.set_pre_delay(0.005);
            let mut energy = 0.0;
            for block in 0..50 {
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
        // Misma sala, distinto material: la lana se la come.
        assert!(tail(0.05) > tail(0.8) * 2.0, "piedra {} vs lana {}", tail(0.05), tail(0.8));
    }

    #[test]
    fn rt60_sets_how_long_the_room_rings() {
        let tail = |room: f32| {
            let mut r = Reverb::new();
            r.set_params(0.5, room, 0.3);
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
        r.set_params(wet, room, 0.3);
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
        r.set_params(1.0, 1.0, 0.0);
        r.set_pre_delay(MAX_PRE_DELAY);
        // Ruido a tope 4 s seguidos: energía en todas las bandas, incluida la que más
        // dura. Una continua no sirve de prueba porque el paso alto se la come.
        let mut seed = 987_654_321u32;
        let mut peak = 0.0f32;
        for _ in 0..200 {
            let input: Vec<f32> = (0..960)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    ((seed >> 16) as f32 / 32_768.0 - 1.0) * 0.8
                })
                .collect();
            let mut out = vec![0.0f32; 1920];
            r.process_add(&input, &mut out);
            peak = out.iter().fold(peak, |m, s| m.max(s.abs()));
        }
        assert!(peak.is_finite() && peak < 4.0, "no debe explotar: {peak}");
    }
}
