//! Remuestreo lineal en streaming (44.1 kHz, 48 kHz... a 48 kHz). Para voz basta.

pub struct LinearResampler {
    step: f64,
    pos: f64,
    pending: Vec<f32>,
}

impl LinearResampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self { step: from_rate as f64 / to_rate as f64, pos: 0.0, pending: Vec::with_capacity(4096) }
    }

    pub fn is_passthrough(&self) -> bool {
        self.step == 1.0
    }

    /// Añade `input` y escribe en `out` todas las muestras que ya se pueden calcular.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_passthrough() {
            out.extend_from_slice(input);
            return;
        }
        self.pending.extend_from_slice(input);
        while self.pos + 1.0 < self.pending.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            let (a, b) = (self.pending[i], self.pending[i + 1]);
            out.push(a + (b - a) * frac);
            self.pos += self.step;
        }
        let consumed = (self.pos as usize).min(self.pending.len());
        self.pending.drain(..consumed);
        self.pos -= consumed as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_is_respected_over_time() {
        let mut r = LinearResampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for _ in 0..100 {
            r.process(&[0.5; 441], &mut out);
        }
        // 44100 muestras de entrada -> ~48000 de salida.
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{}", out.len());
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn downsampling_works_with_tiny_chunks() {
        let mut r = LinearResampler::new(96_000, 48_000);
        let mut out = Vec::new();
        for i in 0..9_600 {
            r.process(&[i as f32], &mut out);
        }
        assert!((out.len() as i64 - 4_800).abs() <= 1);
        // Es una rampa: debe seguir siendo monótona.
        assert!(out.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn passthrough() {
        let mut r = LinearResampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&[1.0, 2.0, 3.0], &mut out);
        assert_eq!(out, vec![1.0, 2.0, 3.0]);
    }
}
