//! Limpieza del micro en tres etapas:
//!
//! 1. Paso alto a 80 Hz: quita zumbido de red (50/60/100 Hz), golpes y retumbe.
//!    RNNoise deja pasar esos zumbidos porque se parecen al tono de una voz.
//! 2. RNNoise (`nnnoiseless`, port en Rust): ruido de fondo, ventiladores, teclado.
//! 3. Puerta de ruido según la probabilidad de voz de RNNoise: -20 dB sin voz.
//!
//! RNNoise trabaja en bloques de 480 muestras (10 ms), así que cada trama de Opus
//! (960) son dos pasadas.

use crate::protocol::{FRAME_SAMPLES, SAMPLE_RATE};
use nnnoiseless::DenoiseState;

const BLOCK: usize = DenoiseState::FRAME_SIZE;
/// RNNoise espera muestras en escala i16, no en [-1, 1].
const SCALE: f32 = 32_768.0;
/// Ganancia de la puerta cuando no hay voz (-20 dB).
const GATE_FLOOR: f32 = 0.1;
/// Por debajo de esta probabilidad de voz la puerta está cerrada,
const GATE_CLOSED: f32 = 0.3;
/// por encima, abierta. Entre ambas, transición suave.
const GATE_OPEN: f32 = 0.6;

/// Biquad paso alto (Audio EQ Cookbook, R. Bristow-Johnson).
struct HighPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl HighPass {
    fn new(cutoff_hz: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / SAMPLE_RATE as f32;
        let alpha = w0.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2); // Q = 0.707 (Butterworth)
        let cos = w0.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos) / 2.0 / a0,
            b1: -(1.0 + cos) / a0,
            b2: (1.0 + cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    highpass: HighPass,
    gate: f32,
    input: [f32; BLOCK],
    output: [f32; BLOCK],
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            highpass: HighPass::new(80.0),
            gate: 1.0,
            input: [0.0; BLOCK],
            output: [0.0; BLOCK],
        }
    }

    /// Apertura actual de la puerta: 0 = cerrada, 1 = abierta.
    pub fn voice_open(&self) -> f32 {
        ((self.gate - GATE_FLOOR) / (1.0 - GATE_FLOOR)).clamp(0.0, 1.0)
    }

    /// Limpia una trama de 20 ms in place. Devuelve la probabilidad de voz (0..1),
    /// la mayor de los dos bloques.
    pub fn process(&mut self, frame: &mut [f32]) -> f32 {
        debug_assert_eq!(frame.len(), FRAME_SAMPLES);
        let mut voice: f32 = 0.0;
        for chunk in frame.chunks_mut(BLOCK) {
            for (dst, src) in self.input.iter_mut().zip(chunk.iter()) {
                *dst = self.highpass.process(*src) * SCALE;
            }
            let p = self.state.process_frame(&mut self.output, &self.input);
            voice = voice.max(p);

            // Abre rápido (no cortar el inicio de las palabras) y cierra despacio.
            let t = ((p - GATE_CLOSED) / (GATE_OPEN - GATE_CLOSED)).clamp(0.0, 1.0);
            let target = GATE_FLOOR + (1.0 - GATE_FLOOR) * t * t * (3.0 - 2.0 * t); // smoothstep
            let step = if target > self.gate { 0.5 } else { 0.05 };
            let start = self.gate;
            let end = self.gate + (target - self.gate) * step;
            self.gate = end;

            let n = chunk.len() as f32;
            for (i, (dst, src)) in chunk.iter_mut().zip(self.output.iter()).enumerate() {
                // Interpolación dentro del bloque para evitar clics.
                let g = start + (end - start) * (i as f32 / n);
                *dst = (*src / SCALE * g).clamp(-1.0, 1.0);
            }
        }
        voice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ruido pseudoaleatorio reproducible (xorshift).
    struct Noise(u32);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
        }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// 3 s de señal; devuelve la reducción en dB después del primer segundo.
    fn reduction_db(mut gen: impl FnMut() -> f32) -> f32 {
        let mut d = Denoiser::new();
        let (mut before, mut after) = (0.0, 0.0);
        for i in 0..150 {
            let mut frame: Vec<f32> = (0..FRAME_SAMPLES).map(|_| gen()).collect();
            let r = rms(&frame);
            d.process(&mut frame);
            if i >= 50 {
                before += r;
                after += rms(&frame);
            }
        }
        20.0 * (after / before).log10()
    }

    #[test]
    fn removes_fan_like_noise() {
        let mut n = Noise(1);
        let mut lp = 0.0f32;
        let db = reduction_db(|| {
            lp = 0.97 * lp + 0.03 * n.next() * 0.3;
            lp
        });
        println!("ventilador: {db:.1} dB");
        assert!(db < -20.0, "{db}");
    }

    #[test]
    fn removes_mains_hum() {
        let mut t = 0usize;
        let db = reduction_db(|| {
            t += 1;
            0.05 * (2.0 * std::f32::consts::PI * 50.0 * t as f32 / SAMPLE_RATE as f32).sin()
        });
        println!("zumbido 50 Hz: {db:.1} dB");
        assert!(db < -15.0, "{db}");
    }

    /// Con ruido blanco RNNoise a veces lo confunde con voz y abre la puerta un
    /// instante, así que se mide la mediana de las tramas y no la media.
    #[test]
    fn reduces_hiss_most_of_the_time() {
        let mut n = Noise(7);
        let mut d = Denoiser::new();
        let mut per_frame_db = Vec::new();
        for i in 0..150 {
            let mut frame: Vec<f32> = (0..FRAME_SAMPLES).map(|_| n.next() * 0.02).collect();
            let r = rms(&frame);
            d.process(&mut frame);
            if i >= 50 {
                per_frame_db.push(20.0 * (rms(&frame) / r).log10());
            }
        }
        per_frame_db.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = per_frame_db[per_frame_db.len() / 2];
        println!("siseo: mediana {median:.1} dB");
        assert!(median < -15.0, "{median}");
    }

    #[test]
    fn highpass_keeps_voice_band() {
        // 1 kHz debe pasar prácticamente intacto.
        let mut hp = HighPass::new(80.0);
        let mut out = Vec::new();
        for t in 0..4800 {
            let x = (2.0 * std::f32::consts::PI * 1000.0 * t as f32 / SAMPLE_RATE as f32).sin();
            out.push(hp.process(x));
        }
        let r = rms(&out[2400..]);
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02, "rms a 1 kHz = {r}");
    }

    #[test]
    fn output_stays_in_range() {
        let mut d = Denoiser::new();
        let mut frame = vec![1.0f32; FRAME_SAMPLES];
        d.process(&mut frame);
        assert!(frame.iter().all(|s| (-1.0..=1.0).contains(s)));
    }
}
