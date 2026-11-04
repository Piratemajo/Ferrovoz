//! Procesado del micro: cancelación de eco (AEC) y ganancia automática (AGC).
//!
//! - AEC: AEC3 de WebRTC (vía `sonora`, port en Rust, BSD-3). Quita del micro lo
//!   que sale por los altavoces, para que quien juega sin cascos no reenvíe la voz
//!   de los demás. La referencia es todo lo que Ferrovoz manda a la salida salvo el
//!   monitor de micro.
//! - AGC: [`AutoGain`], +-12 dB.
//!
//! Orden: AEC -> RNNoise -> AGC. La ganancia va después del denoiser para no
//! amplificar lo que se cuela con cascos.
//!
//! Sin la feature `audio` (librería del servidor) no hace nada.

use crate::autogain::AutoGain;
#[cfg(feature = "audio")]
use crate::protocol::FRAME_SAMPLES;

/// WebRTC trabaja en bloques de 10 ms; las tramas son de 20 ms.
#[cfg(feature = "audio")]
const BLOCK: usize = FRAME_SAMPLES / 2;

pub struct MicProcessor {
    echo_cancel: bool,
    auto_gain: bool,
    #[cfg(feature = "audio")]
    echo: Option<sonora::AudioProcessing>,
    gain: Option<AutoGain>,
}

impl MicProcessor {
    pub fn new(echo_cancel: bool, auto_gain: bool) -> Self {
        let mut p = Self {
            echo_cancel: !echo_cancel, // fuerza a `configure` a crearlo
            auto_gain,
            #[cfg(feature = "audio")]
            echo: None,
            gain: None,
        };
        p.configure(echo_cancel, auto_gain);
        p
    }

    pub fn echo_cancel(&self) -> bool {
        self.echo_cancel
    }

    pub fn auto_gain(&self) -> bool {
        self.auto_gain
    }

    /// Cambia los ajustes. Si cambian, se reinicia (el AEC tarda ~1 s en converger).
    pub fn configure(&mut self, echo_cancel: bool, auto_gain: bool) {
        if echo_cancel == self.echo_cancel && auto_gain == self.auto_gain {
            return;
        }
        self.echo_cancel = echo_cancel;
        self.auto_gain = auto_gain;
        #[cfg(feature = "audio")]
        {
            self.echo = echo_cancel.then(build_echo);
        }
        self.gain = auto_gain.then(AutoGain::new);
    }

    /// Señal de salida (mono, 20 ms), referencia del AEC.
    #[cfg(feature = "audio")]
    pub fn render(&mut self, frame: &[f32]) {
        if !self.echo_cancel {
            return;
        }
        if let Some(apm) = &mut self.echo {
            let mut sink = [0f32; BLOCK];
            for block in frame.chunks_exact(BLOCK) {
                let _ = apm.process_render_f32(&[block], &mut [&mut sink]);
            }
        }
    }

    /// Paso 1, antes del denoiser: AEC (mono, 20 ms, in place).
    #[cfg(feature = "audio")]
    pub fn cancel_echo(&mut self, frame: &mut [f32]) {
        run(&mut self.echo, frame);
    }

    /// Último paso, después del denoiser: AGC (ver [`AutoGain`]).
    /// `voice_prob`: probabilidad de voz de RNNoise (None si está apagado).
    /// `voice_open`: apertura de la puerta de ruido (0 en las pausas).
    pub fn adjust_gain(&mut self, frame: &mut [f32], voice_prob: Option<f32>, voice_open: f32) {
        if let Some(agc) = &mut self.gain {
            agc.process(frame, voice_prob, voice_open);
        }
    }

    /// AEC y AGC seguidos, sin denoiser.
    pub fn capture(&mut self, frame: &mut [f32]) {
        self.cancel_echo(frame);
        self.adjust_gain(frame, None, 1.0);
    }

    /// Sin audio (librería del servidor): nada.
    #[cfg(not(feature = "audio"))]
    pub fn render(&mut self, _frame: &[f32]) {}

    #[cfg(not(feature = "audio"))]
    pub fn cancel_echo(&mut self, _frame: &mut [f32]) {}

}

#[cfg(feature = "audio")]
fn run(apm: &mut Option<sonora::AudioProcessing>, frame: &mut [f32]) {
    if let Some(apm) = apm {
        let mut out = [0f32; BLOCK];
        for block in frame.chunks_exact_mut(BLOCK) {
            if apm.process_capture_f32(&[&*block], &mut [&mut out]).is_ok() {
                block.copy_from_slice(&out);
            }
        }
    }
}

#[cfg(feature = "audio")]
fn build(config: sonora::Config) -> sonora::AudioProcessing {
    use sonora::{AudioProcessing, StreamConfig};
    AudioProcessing::builder()
        .config(config)
        .capture_config(StreamConfig::new(crate::protocol::SAMPLE_RATE, 1))
        .render_config(StreamConfig::new(crate::protocol::SAMPLE_RATE, 1))
        .build()
}

/// AEC con el paso alto que necesita.
#[cfg(feature = "audio")]
fn build_echo() -> sonora::AudioProcessing {
    use sonora::config::{EchoCanceller, HighPassFilter};
    build(sonora::Config {
        high_pass_filter: Some(HighPassFilter::default()),
        echo_canceller: Some(EchoCanceller::default()),
        ..sonora::Config::default()
    })
}

#[cfg(all(test, feature = "audio"))]
pub(crate) mod tests {
    use super::*;

    /// Señal parecida a voz: pulsos glotales con tono variable por tres formantes que
    /// cambian en cada sílaba, ~4 sílabas/s con pausas.
    pub(crate) fn voice_like(n: usize, seed: u32) -> Vec<f32> {
        use std::f32::consts::PI;
        let sr = 48_000.0f32;
        let vowels = [[700.0, 1220.0, 2600.0], [300.0, 2300.0, 3000.0], [500.0, 900.0, 2400.0], [400.0, 1900.0, 2550.0]];
        let mut x = seed;
        let mut rnd = move || {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) as f32 / (1u32 << 24) as f32
        };
        // Resonadores de 2º orden.
        let mut st = [[0f32; 2]; 3];
        let mut phase = 0f32;
        let syl = (sr * 0.25) as usize;
        let mut out = Vec::with_capacity(n);
        let mut vowel = vowels[0];
        let mut pitch = 140.0;
        let mut voiced = true;
        for i in 0..n {
            if i % syl == 0 {
                vowel = vowels[(rnd() * 4.0) as usize % 4];
                pitch = 110.0 + rnd() * 90.0;
                voiced = rnd() > 0.2; // pausas
            }
            let pos = (i % syl) as f32 / syl as f32;
            let env = if voiced { (pos * PI).sin().powf(0.6) } else { 0.0 };
            phase += pitch / sr;
            let excite = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            } + (rnd() - 0.5) * 0.05;
            let mut y = 0f32;
            for (k, f) in vowel.iter().enumerate() {
                let r = 0.995f32 - k as f32 * 0.003;
                let c = 2.0 * r * (2.0 * PI * f / sr).cos();
                let v = excite + c * st[k][0] - r * r * st[k][1];
                st[k][1] = st[k][0];
                st[k][0] = v;
                y += v * [1.0, 0.6, 0.3][k];
            }
            out.push(y * env * 0.01);
        }
        out
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32
    }

    /// Altavoz -> micro con 40 ms de retraso y algo de sala: tras converger el eco baja mucho.
    #[test]
    fn echo_from_the_speakers_is_removed() {
        let mut p = MicProcessor::new(true, false);
        let seconds = 6;
        let far = voice_like(48_000 * seconds, 1);
        let delay = 48 * 40;
        let mut before = 0f32;
        let mut after = 0f32;
        for (i, chunk) in far.chunks_exact(FRAME_SAMPLES).enumerate() {
            p.render(chunk);
            let start = i * FRAME_SAMPLES;
            let mut mic: Vec<f32> = (0..FRAME_SAMPLES)
                .map(|k| {
                    let t = start + k;
                    let a = if t >= delay { far[t - delay] } else { 0.0 };
                    let b = if t >= delay + 240 { far[t - delay - 240] } else { 0.0 };
                    0.5 * a + 0.2 * b
                })
                .collect();
            if start >= 48_000 * (seconds - 2) {
                before += energy(&mic);
                p.capture(&mut mic);
                after += energy(&mic);
            } else {
                p.capture(&mut mic);
            }
        }
        let erle_db = 10.0 * (before / after.max(1e-12)).log10();
        eprintln!("eco reducido {erle_db:.1} dB");
        assert!(erle_db > 15.0, "solo {erle_db:.1} dB menos de eco");
    }

    /// Double talk: el eco se va y la voz local se queda.
    #[test]
    fn double_talk_keeps_your_voice() {
        let mut p = MicProcessor::new(true, false);
        let far = voice_like(48_000 * 6, 11);
        let near = voice_like(48_000 * 6, 23);
        let delay = 48 * 40;
        let (mut near_e, mut out_e, mut echo_e) = (0f32, 0f32, 0f32);
        for (i, chunk) in far.chunks_exact(FRAME_SAMPLES).enumerate() {
            p.render(chunk);
            let start = i * FRAME_SAMPLES;
            let echo: Vec<f32> = (0..FRAME_SAMPLES)
                .map(|k| if start + k >= delay { 0.5 * far[start + k - delay] } else { 0.0 })
                .collect();
            // 3 s solo de eco para converger; después habla también el local.
            let talking = start >= 48_000 * 3;
            let mut mic: Vec<f32> = (0..FRAME_SAMPLES)
                .map(|k| echo[k] + if talking { near[start + k] } else { 0.0 })
                .collect();
            p.capture(&mut mic);
            if start >= 48_000 * 4 {
                near_e += energy(&near[start..start + FRAME_SAMPLES]);
                echo_e += energy(&echo);
                out_e += energy(&mic);
            }
        }
        let kept_db = 10.0 * (out_e / near_e).log10();
        eprintln!("doble conversación: tu voz {kept_db:+.1} dB, eco era {:+.1} dB respecto a tu voz", 10.0 * (echo_e / near_e).log10());
        assert!(kept_db > -6.0, "tu voz baja {kept_db:.1} dB");
    }

    /// Sin nada por los altavoces la voz pasa intacta.
    #[test]
    fn your_own_voice_passes_through() {
        let mut p = MicProcessor::new(true, false);
        let near = voice_like(48_000 * 3, 7);
        let silence = [0f32; FRAME_SAMPLES];
        let (mut before, mut after) = (0f32, 0f32);
        for chunk in near.chunks_exact(FRAME_SAMPLES) {
            p.render(&silence);
            let mut mic = chunk.to_vec();
            before += energy(&mic);
            p.capture(&mut mic);
            after += energy(&mic);
        }
        let loss_db = 10.0 * (before / after.max(1e-12)).log10();
        eprintln!("voz propia: {loss_db:.1} dB de pérdida");
        assert!(loss_db < 3.0, "la voz pierde {loss_db:.1} dB");
    }

    /// Un micro muy bajo sube y uno muy alto no satura.
    #[test]
    fn auto_gain_evens_out_quiet_and_loud_mics() {
        let level = |scale: f32| {
            let mut p = MicProcessor::new(false, true);
            let voice = voice_like(48_000 * 8, 3);
            let mut last = 0f32;
            for (i, chunk) in voice.chunks_exact(FRAME_SAMPLES).enumerate() {
                let mut mic: Vec<f32> = chunk.iter().map(|s| s * scale).collect();
                p.capture(&mut mic);
                if i * FRAME_SAMPLES >= 48_000 * 6 {
                    last += energy(&mic);
                }
            }
            10.0 * last.max(1e-12).log10()
        };
        let quiet_in = 10.0 * {
            let v = voice_like(48_000 * 8, 3);
            v.iter().map(|s| (s * 0.03).powi(2)).sum::<f32>() / v.len() as f32 * 100.0
        }
        .log10();
        let quiet = level(0.03);
        let loud = level(3.0);
        eprintln!("micro bajo: {quiet:.1} dB (entrada {quiet_in:.1}), micro alto: {loud:.1} dB");
        assert!(quiet > quiet_in + 6.0, "no ha subido el micro bajo");
        // Entran con 40 dB de diferencia; con +-12 dB se reduce al menos 12 dB.
        assert!(loud - quiet < 28.0, "siguen muy distintos: {quiet:.1} vs {loud:.1}");
    }

    #[test]
    fn off_means_untouched() {
        let mut p = MicProcessor::new(false, false);
        let mut frame = voice_like(FRAME_SAMPLES, 5);
        let copy = frame.clone();
        p.render(&copy);
        p.capture(&mut frame);
        assert_eq!(frame, copy);
    }
}
