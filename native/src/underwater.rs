//! Efecto submarino.
//!
//! - Paso bajo de 4º orden (dos biquads) con algo de resonancia cerca del corte,
//!   que da el tono hueco.
//! - Vibrato irregular: línea de retardo modulada por dos osciladores lentos.
//! - Trémolo suave algo más rápido.
//!
//! La cantidad cambia con una rampa de ~120 ms para evitar clics. Con 0 la señal
//! no se toca.

use crate::protocol::SAMPLE_RATE;
use std::f32::consts::TAU;

/// Cantidad cuando solo uno (hablante u oyente) está bajo el agua.
pub const ONE_SIDE: f32 = 0.75;
/// Cantidad cuando lo están los dos.
pub const BOTH: f32 = 1.0;

const DELAY_LEN: usize = 1024; // ~21 ms a 48 kHz
const BASE_DELAY_MS: f32 = 7.0;

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// Paso bajo (RBJ) con resonancia `q`.
    fn set_lowpass(&mut self, cutoff: f32, q: f32) {
        let w = TAU * cutoff / SAMPLE_RATE as f32;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - c) / 2.0 / a0;
        self.b1 = (1.0 - c) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * c / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        // Forma directa II transpuesta
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

pub struct Underwater {
    lp: [Biquad; 2],
    delay: Box<[f32; DELAY_LEN]>,
    pos: usize,
    /// Fases de los osciladores (0..1).
    wobble1: f32,
    wobble2: f32,
    bubble: f32,
    /// Cantidad actual (suavizada) y la usada en el último cálculo de filtros.
    amount: f32,
    tuned_for: f32,
}

impl Default for Underwater {
    fn default() -> Self {
        Self::new()
    }
}

impl Underwater {
    pub fn new() -> Self {
        Self {
            lp: [Biquad::default(); 2],
            delay: Box::new([0.0; DELAY_LEN]),
            pos: 0,
            wobble1: 0.0,
            wobble2: 0.37,
            bubble: 0.0,
            amount: 0.0,
            tuned_for: -1.0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.amount > 0.0
    }

    /// Corte según la cantidad: ~3 kHz, ~660 Hz con uno bajo el agua, ~400 Hz con los dos.
    fn cutoff(amount: f32) -> f32 {
        3000.0 * (400.0f32 / 3000.0).powf(amount.clamp(0.0, 1.0))
    }

    /// Procesa `pcm` (mono, 48 kHz) in place. `target` en 0..=1 (0, [`ONE_SIDE`], [`BOTH`]).
    pub fn process(&mut self, pcm: &mut [f32], target: f32) {
        let target = target.clamp(0.0, 1.0);
        if target == 0.0 && self.amount == 0.0 {
            return; // sin efecto
        }
        // Rampa de ~120 ms.
        let step = 1.0 / (0.12 * SAMPLE_RATE as f32);
        // Filtros recalculados por trama con la cantidad actual.
        let tune_at = self.amount.max(target).max(0.05);
        if (tune_at - self.tuned_for).abs() > 0.01 {
            let fc = Self::cutoff(tune_at);
            self.lp[0].set_lowpass(fc, 1.1); // algo de resonancia
            self.lp[1].set_lowpass(fc, 0.6);
            self.tuned_for = tune_at;
        }
        let sr = SAMPLE_RATE as f32;
        let (w1, w2, wb) = (0.8 / sr, 2.9 / sr, 5.5 / sr);
        for s in pcm.iter_mut() {
            let dry = *s;
            self.amount = if self.amount < target {
                (self.amount + step).min(target)
            } else {
                (self.amount - step).max(target)
            };

            // Vibrato: +-2.2 ms lento y +-0.6 ms más rápido.
            self.delay[self.pos] = dry;
            let d_ms = BASE_DELAY_MS + 2.2 * (TAU * self.wobble1).sin() + 0.6 * (TAU * self.wobble2).sin();
            let d = d_ms * sr / 1000.0;
            let read = self.pos as f32 + DELAY_LEN as f32 - d;
            let i0 = read.floor() as usize % DELAY_LEN;
            let frac = read - read.floor();
            let i1 = (i0 + 1) % DELAY_LEN;
            let wobbled = self.delay[i0] * (1.0 - frac) + self.delay[i1] * frac;
            self.pos = (self.pos + 1) % DELAY_LEN;

            let first = self.lp[0].run(wobbled);
            let muffled = self.lp[1].run(first);
            let bubbles = 1.0 - 0.18 * (0.5 + 0.5 * (TAU * self.bubble).sin());
            let wet = muffled * bubbles * 0.85;

            // Con un lado bajo el agua ya es efecto completo.
            let mix = (self.amount / ONE_SIDE).min(1.0);
            *s = dry * (1.0 - mix) + wet * mix;

            self.wobble1 = (self.wobble1 + w1).fract();
            self.wobble2 = (self.wobble2 + w2).fract();
            self.bubble = (self.bubble + wb).fract();
        }
        if self.amount == 0.0 {
            // Fuera del agua: reset.
            self.lp.iter_mut().for_each(Biquad::reset);
            self.delay.fill(0.0);
            self.tuned_for = -1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::FRAME_SAMPLES;

    fn tone(freq: f32, frames: usize) -> Vec<f32> {
        (0..FRAME_SAMPLES * frames)
            .map(|i| 0.5 * (TAU * freq * i as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    /// Energía tras `frames` tramas, sin el primer medio segundo (rampa).
    fn energy_after(freq: f32, target: f32) -> f32 {
        let mut u = Underwater::new();
        let mut x = tone(freq, 75);
        for f in x.chunks_mut(FRAME_SAMPLES) {
            u.process(f, target);
        }
        x[FRAME_SAMPLES * 25..].iter().map(|s| s * s).sum()
    }

    #[test]
    fn off_means_untouched() {
        let mut u = Underwater::new();
        let original = tone(1000.0, 3);
        let mut x = original.clone();
        for f in x.chunks_mut(FRAME_SAMPLES) {
            u.process(f, 0.0);
        }
        assert_eq!(x, original, "sin agua la voz sale idéntica");
    }

    #[test]
    fn treble_disappears_but_the_voice_body_stays() {
        let db = |a: f32, b: f32| 10.0 * (a / b).log10();
        let (hi_dry, lo_dry) = (energy_after(3000.0, 0.0), energy_after(300.0, 0.0));
        let (hi, lo) = (energy_after(3000.0, ONE_SIDE), energy_after(300.0, ONE_SIDE));
        let (hi2, _) = (energy_after(3000.0, BOTH), 0.0);
        println!(
            "3 kHz: {:.1} dB (uno) / {:.1} dB (los dos) · 300 Hz: {:.1} dB",
            db(hi, hi_dry),
            db(hi2, hi_dry),
            db(lo, lo_dry)
        );
        assert!(db(hi, hi_dry) < -25.0, "los agudos casi desaparecen");
        assert!(db(lo, lo_dry) > -4.0, "los graves (el cuerpo de la voz) se mantienen");
        assert!(hi2 < hi, "con los dos bajo el agua, aún más tapado");
    }

    #[test]
    fn pitch_wobbles_like_moving_water() {
        // Tono de 300 Hz: periodo entre cruces por cero.
        let periods = |target: f32| {
            let mut u = Underwater::new();
            let mut x = tone(300.0, 150);
            for f in x.chunks_mut(FRAME_SAMPLES) {
                u.process(f, target);
            }
            let tail = &x[FRAME_SAMPLES * 25..];
            let mut crossings = Vec::new();
            for i in 1..tail.len() {
                if tail[i - 1] <= 0.0 && tail[i] > 0.0 {
                    let t = (i - 1) as f32 + tail[i - 1] / (tail[i - 1] - tail[i]);
                    crossings.push(t);
                }
            }
            let p: Vec<f32> = crossings.windows(2).map(|w| w[1] - w[0]).collect();
            let mean = p.iter().sum::<f32>() / p.len() as f32;
            (p.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / p.len() as f32).sqrt() / mean
        };
        let (dry, wet) = (periods(0.0), periods(ONE_SIDE));
        println!("variación del tono: seco {:.4} %, bajo el agua {:.3} %", dry * 100.0, wet * 100.0);
        assert!(wet > 0.003 && wet > dry * 20.0, "el tono ondula bajo el agua");
        assert!(wet < 0.05, "pero sin desafinar de forma exagerada");
    }

    #[test]
    fn entering_and_leaving_water_does_not_click() {
        let mut u = Underwater::new();
        let mut x = tone(300.0, 60);
        let mut worst = 0f32;
        for (n, f) in x.chunks_mut(FRAME_SAMPLES).enumerate() {
            // Entra a los 200 ms y sale a los 700 ms.
            let target = if (10..35).contains(&n) { ONE_SIDE } else { 0.0 };
            u.process(f, target);
        }
        for w in x.windows(2) {
            worst = worst.max((w[1] - w[0]).abs());
        }
        // Un seno de 300 Hz y amplitud 0.5 cambia como mucho ~0.02 por muestra.
        assert!(worst < 0.05, "sin saltos bruscos: {worst}");
        assert!(!u.is_active(), "al salir, el efecto se apaga del todo");
    }
}
