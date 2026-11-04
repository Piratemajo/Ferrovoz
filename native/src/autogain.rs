//! Ganancia automática del micro.
//!
//! Lleva la voz a un nivel normal sin subir lo que se cuela (con cascos, el sonido
//! del juego; ventiladores; otras voces en la habitación):
//!
//! - Va después del denoiser y solo aprende de tramas con voz.
//! - El nivel de referencia es el percentil 95 de las tramas con voz de los últimos
//!   15 s. La voz propia, cerca del micro, es lo más fuerte; lo que se cuela suele
//!   estar bastante más bajo y no mueve la medida aunque RNNoise lo tome por voz.
//! - Entre -12 y +12 dB; sube a 3 dB/s y baja a 15 dB/s.
//! - No se aplica en las pausas (puerta cerrada).
//! - Limitador a -1 dBFS.

use std::collections::VecDeque;

/// Nivel objetivo (RMS de las tramas fuertes, dBFS).
const TARGET_DBFS: f32 = -18.0;
const MAX_GAIN_DB: f32 = 12.0;
const MIN_GAIN_DB: f32 = -12.0;
/// Por trama de 20 ms: 3 dB/s al subir, 15 dB/s al bajar.
const UP_DB_PER_FRAME: f32 = 0.06;
const DOWN_DB_PER_FRAME: f32 = 0.3;
/// Tramas con voz en el historial (15 s) y mínimo para empezar.
const HISTORY: usize = 750;
const MIN_HISTORY: usize = 25;
/// Probabilidad de voz (RNNoise) mínima para que una trama cuente.
const VOICE_PROB: f32 = 0.6;
/// Sin denoiser no hay probabilidad: cuenta lo que pase de este nivel.
const VOICE_FLOOR_DBFS: f32 = -55.0;
/// Pico máximo tras la ganancia (-1 dBFS).
const PEAK_LIMIT: f32 = 0.89;

pub struct AutoGain {
    history: VecDeque<f32>,
    /// Nivel estimado de la voz (dBFS), None mientras no hay datos suficientes.
    speech_db: Option<f32>,
    frames_since_estimate: usize,
    gain_db: f32,
}

impl Default for AutoGain {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoGain {
    pub fn new() -> Self {
        Self { history: VecDeque::with_capacity(HISTORY), speech_db: None, frames_since_estimate: 0, gain_db: 0.0 }
    }

    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// Procesa una trama in place.
    ///
    /// - `voice_prob`: probabilidad de voz del denoiser (None si está apagado).
    /// - `voice_open`: apertura de la puerta de ruido (0 = pausa, 1 = voz).
    pub fn process(&mut self, frame: &mut [f32], voice_prob: Option<f32>, voice_open: f32) {
        if frame.is_empty() {
            return;
        }
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        let level_db = 20.0 * rms.max(1e-9).log10();
        let is_voice = match voice_prob {
            Some(p) => p >= VOICE_PROB,
            None => level_db > VOICE_FLOOR_DBFS,
        };

        if is_voice {
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(level_db);
            self.frames_since_estimate += 1;
            // Percentil cada 5 tramas.
            if self.history.len() >= MIN_HISTORY && (self.speech_db.is_none() || self.frames_since_estimate >= 5) {
                let mut sorted: Vec<f32> = self.history.iter().copied().collect();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                self.speech_db = Some(sorted[(sorted.len() * 19) / 20]);
                self.frames_since_estimate = 0;
            }
        }

        // Hacia el objetivo: despacio al subir, rápido al bajar.
        let start_db = self.gain_db;
        if let Some(speech) = self.speech_db {
            let target = (TARGET_DBFS - speech).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
            self.gain_db = if target > self.gain_db {
                (self.gain_db + UP_DB_PER_FRAME).min(target)
            } else {
                (self.gain_db - DOWN_DB_PER_FRAME).max(target)
            };
        }

        // Limitador a -1 dBFS.
        let peak = frame.iter().fold(0f32, |m, s| m.max(s.abs()));
        let to_linear = |db: f32| 10f32.powf(db / 20.0);
        let mut end = to_linear(self.gain_db);
        if peak * end > PEAK_LIMIT {
            end = PEAK_LIMIT / peak;
        }
        let start = to_linear(start_db).min(PEAK_LIMIT / peak.max(1e-9));

        // En las pausas, nivel original.
        let open = voice_open.clamp(0.0, 1.0);
        let n = frame.len() as f32;
        for (i, s) in frame.iter_mut().enumerate() {
            let g = start + (end - start) * ((i + 1) as f32 / n);
            *s *= 1.0 + (g - 1.0) * open;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Voz": ráfagas de un tono con envolvente (0.25 s sonando, 0.1 s pausa).
    fn speech_frame(f: usize, level: f32) -> Vec<f32> {
        (0..960)
            .map(|k| {
                let t = (f * 960 + k) as f32 / 48_000.0;
                let syllable = (t * 2.857).fract() < 0.7;
                if syllable {
                    level * (2.0 * std::f32::consts::PI * 180.0 * t).sin() * (std::f32::consts::PI * (t * 2.857).fract() / 0.7).sin()
                } else {
                    0.0
                }
            })
            .collect()
    }

    fn rms_db(x: &[f32]) -> f32 {
        10.0 * (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).max(1e-20).log10()
    }

    fn run(level: f32, frames: usize) -> (AutoGain, f32) {
        let mut agc = AutoGain::new();
        let mut out_db = 0.0;
        for f in 0..frames {
            let mut fr = speech_frame(f, level);
            agc.process(&mut fr, Some(0.9), 1.0);
            out_db = rms_db(&fr);
        }
        (agc, out_db)
    }

    #[test]
    fn quiet_mic_goes_up_loud_mic_goes_down() {
        let (quiet, _) = run(0.01, 500); // ~ -40 dBFS
        let (loud, _) = run(0.9, 500); // ~ -4 dBFS
        let (normal, _) = run(0.17, 500); // ~ -18 dBFS en las tramas fuertes
        assert!((quiet.gain_db() - MAX_GAIN_DB).abs() < 0.1, "bajo: {}", quiet.gain_db());
        assert!(loud.gain_db() < -10.0, "alto: {}", loud.gain_db());
        assert!(normal.gain_db().abs() < 3.0, "normal: {}", normal.gain_db());
    }

    #[test]
    fn rises_slowly_falls_fast() {
        // A 1 s de empezar todavía no ha subido todo (3 dB/s).
        let (agc, _) = run(0.01, 75);
        assert!(agc.gain_db() > 1.0 && agc.gain_db() < 4.0, "{}", agc.gain_db());
    }

    #[test]
    fn something_quieter_than_you_does_not_change_your_level() {
        // Voz el 30 % del tiempo; el resto, algo que RNNoise toma por voz 24 dB más
        // bajo. La ganancia debe quedar como con la voz sola.
        let alone = run(0.12, 1500).0.gain_db();
        let mut agc = AutoGain::new();
        for f in 0..1500 {
            let you = (f / 50) % 10 < 3;
            let mut fr = speech_frame(f, if you { 0.12 } else { 0.0075 });
            agc.process(&mut fr, Some(0.9), 1.0);
        }
        assert!((agc.gain_db() - alone).abs() < 1.5, "con música {:.1} dB, solo tú {alone:.1} dB", agc.gain_db());
    }

    #[test]
    fn pauses_are_left_alone() {
        let (mut agc, _) = run(0.01, 500); // ya a +12 dB
        let mut fr = speech_frame(3, 0.01);
        let before = rms_db(&fr);
        agc.process(&mut fr, Some(0.1), 0.0);
        assert!((rms_db(&fr) - before).abs() < 0.01);
    }

    #[test]
    fn noise_alone_never_raises_the_gain() {
        let mut agc = AutoGain::new();
        for f in 0..500 {
            let mut fr = speech_frame(f, 0.003);
            agc.process(&mut fr, Some(0.1), 0.2); // RNNoise: no es voz
        }
        assert_eq!(agc.gain_db(), 0.0);
    }

    #[test]
    fn a_shout_does_not_clip() {
        let (mut agc, _) = run(0.01, 500); // +12 dB
        let mut fr = speech_frame(3, 0.8);
        agc.process(&mut fr, Some(0.9), 1.0);
        assert!(fr.iter().all(|s| s.abs() <= PEAK_LIMIT + 1e-3), "pico {}", fr.iter().fold(0f32, |m, s| m.max(s.abs())));
    }
}
