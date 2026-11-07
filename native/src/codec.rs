//! Wrapper de libopus (compilada desde fuente por `audiopus_sys`).

use crate::protocol::{FRAME_SAMPLES, MAX_PACKET, SAMPLE_RATE};
use opus::{Application, Bitrate, Channels};

pub struct VoiceEncoder {
    inner: opus::Encoder,
    out: Vec<u8>,
}

/// Tamaño máximo de un paquete Opus de 20 ms (RFC 6716).
pub const MAX_OPUS_FRAME: usize = 1275;

impl VoiceEncoder {
    /// Para voz.
    pub fn new(bitrate: i32) -> Result<Self, opus::Error> {
        Self::with_application(bitrate, false)
    }

    /// `music = true` para música y efectos (modo `Audio` de Opus): menos agresivo con
    /// lo que no es voz.
    pub fn with_application(bitrate: i32, music: bool) -> Result<Self, opus::Error> {
        let app = if music { Application::Audio } else { Application::Voip };
        let mut inner = opus::Encoder::new(SAMPLE_RATE, Channels::Mono, app)?;
        inner.set_bitrate(Bitrate::Bits(bitrate))?;
        // FEC en banda: cada paquete lleva una copia de baja calidad del anterior.
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(10)?;
        Ok(Self { inner, out: vec![0u8; MAX_PACKET] })
    }

    /// Bitrate y FEC en marcha.
    pub fn tune(&mut self, bitrate: i32, expected_loss_percent: i32) {
        let _ = self.inner.set_bitrate(Bitrate::Bits(bitrate));
        let _ = self.inner.set_packet_loss_perc(expected_loss_percent.clamp(0, 100));
    }

    /// Complejidad de Opus (0-10): menos es más rápido y algo peor.
    pub fn set_complexity(&mut self, complexity: i32) {
        let _ = self.inner.set_complexity(complexity.clamp(0, 10));
    }

    /// Codifica una trama de 20 ms (960 muestras mono f32).
    pub fn encode(&mut self, pcm: &[f32]) -> Result<&[u8], opus::Error> {
        debug_assert_eq!(pcm.len(), FRAME_SAMPLES);
        let n = self.inner.encode_float(pcm, &mut self.out)?;
        Ok(&self.out[..n])
    }
}

pub struct VoiceDecoder {
    inner: opus::Decoder,
}

impl VoiceDecoder {
    pub fn new() -> Result<Self, opus::Error> {
        Ok(Self { inner: opus::Decoder::new(SAMPLE_RATE, Channels::Mono)? })
    }

    /// Decodifica una trama normal.
    pub fn decode(&mut self, packet: &[u8], out: &mut [f32; FRAME_SAMPLES]) -> usize {
        self.inner.decode_float(packet, out, false).unwrap_or_else(|_| {
            out.fill(0.0);
            FRAME_SAMPLES
        })
    }

    /// Reconstruye una trama perdida a partir del FEC que trae la siguiente.
    pub fn decode_fec(&mut self, next_packet: &[u8], out: &mut [f32; FRAME_SAMPLES]) -> usize {
        self.inner.decode_float(next_packet, out, true).unwrap_or_else(|_| self.conceal(out))
    }

    /// Ocultación de pérdidas (PLC) cuando no hay nada mejor.
    pub fn conceal(&mut self, out: &mut [f32; FRAME_SAMPLES]) -> usize {
        self.inner.decode_float(&[], out, false).unwrap_or_else(|_| {
            out.fill(0.0);
            FRAME_SAMPLES
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, phase0: usize) -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|i| 0.4 * (2.0 * std::f32::consts::PI * freq * (phase0 + i) as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    #[test]
    fn encode_decode_preserves_energy() {
        let mut enc = VoiceEncoder::new(32_000).unwrap();
        let mut dec = VoiceDecoder::new().unwrap();
        let mut out = [0f32; FRAME_SAMPLES];
        let mut energy = 0.0;
        for f in 0..10 {
            let pkt = enc.encode(&sine(440.0, f * FRAME_SAMPLES)).unwrap().to_vec();
            assert!(pkt.len() < 200, "una trama de voz debería ocupar poco: {}", pkt.len());
            assert_eq!(dec.decode(&pkt, &mut out), FRAME_SAMPLES);
            if f >= 3 {
                energy += out.iter().map(|s| s * s).sum::<f32>();
            }
        }
        let rms = (energy / (7.0 * FRAME_SAMPLES as f32)).sqrt();
        // Seno de amplitud 0.4 -> RMS ~0.283
        assert!(rms > 0.15 && rms < 0.4, "rms = {rms}");
    }

    #[test]
    fn conceal_produces_a_frame() {
        let mut dec = VoiceDecoder::new().unwrap();
        let mut out = [1f32; FRAME_SAMPLES];
        assert_eq!(dec.conceal(&mut out), FRAME_SAMPLES);
    }
}
