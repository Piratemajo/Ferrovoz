//! Binaural para cascos: además de izquierda/derecha, delante/detrás y arriba/abajo.
//!
//! Modelo estructural de cabeza y oreja (Brown y Duda, 1998), sin HRIR medidas:
//! barato, sin datos con licencia y sin saltos al moverse. Por hablante:
//!
//! 1. Pinna: ecos de pocas muestras cuyo retardo depende de la elevación y de
//!    delante/detrás. Generan las muescas en agudos que dan la altura.
//! 2. Sombra trasera: menos agudos para lo que viene de atrás.
//! 3. Sombra de la cabeza: filtro de un polo por oído.
//! 4. ITD: hasta ~0.7 ms de diferencia entre oídos (Woodworth).
//!
//! Coordenadas de la cabeza: x = derecha, y = arriba, z = delante.

use crate::protocol::SAMPLE_RATE;

const FS: f32 = SAMPLE_RATE as f32;
/// Radio medio de la cabeza (m) y velocidad del sonido (m/s).
const HEAD_RADIUS: f32 = 0.0875;
const SPEED_OF_SOUND: f32 = 343.0;
/// a/c en muestras (~12 a 48 kHz).
const HEAD_SAMPLES: f32 = HEAD_RADIUS / SPEED_OF_SOUND * FS;

/// Ecos de la pinna (Brown y Duda): amplitud, A, B (muestras a 44.1 kHz) y D.
const PINNA_RHO: [f32; 5] = [0.5, -1.0, 0.5, -0.25, 0.25];
const PINNA_A: [f32; 5] = [1.0, 5.0, 5.0, 5.0, 5.0];
const PINNA_B: [f32; 5] = [2.0, 4.0, 7.0, 11.0, 13.0];
const PINNA_D: f32 = 0.85;
/// Mezcla de los ecos de la pinna (1 = modelo original; algo menos suena más natural).
const PINNA_MIX: f32 = 0.6;

/// Sombra de la cabeza: alfa mínimo y ángulo (desde el eje del oído) donde se da.
const ALPHA_MIN: f32 = 0.1;
const THETA_MIN: f32 = 150.0;
const ALPHA_MAX: f32 = 1.5;

/// Atenuación de agudos para lo que viene justo de detrás (0.5 = -6 dB sobre ~3 kHz).
const REAR_SHADE: f32 = 0.5;
const REAR_CUTOFF: f32 = 3_000.0;

/// Compensación: el modelo apaga algo los agudos de lo que está delante. Con esto el
/// volumen percibido es como el del paneo simple.
const MAKEUP: f32 = 1.12;

const DELAY_LEN: usize = 64; // > ITD máximo (~32) y ecos de la oreja (~21)

/// Dirección del hablante respecto a la cabeza (unitaria: x derecha, y arriba, z delante).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Direction {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Direction {
    pub const FRONT: Self = Self { x: 0.0, y: 0.0, z: 1.0 };

    /// `rel` = hablante - oyente (coordenadas del mundo); yaw y pitch del oyente en grados
    /// (yaw 0 = mirando a +Z; pitch positivo = abajo). `None` si está justo encima.
    pub fn from_world(rel: [f32; 3], yaw_deg: f32, pitch_deg: f32) -> Option<Self> {
        let len = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
        if len < 0.3 {
            return None;
        }
        let d = [rel[0] / len, rel[1] / len, rel[2] / len];
        let (sy, cy) = yaw_deg.to_radians().sin_cos();
        let (sp, cp) = pitch_deg.to_radians().sin_cos();
        let forward = [-sy * cp, -sp, cy * cp];
        let right = [-cy, 0.0, -sy];
    // up = right x forward
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        Some(Self { x: dot(right), y: dot(up), z: dot(forward) })
    }
}

/// Parámetros que solo dependen de la dirección (una vez por trama).
#[derive(Debug, Clone, Copy)]
struct Params {
    /// Retardo de cada eco de la pinna, en muestras a 48 kHz.
    pinna: [f32; 5],
    /// 0 = delante, 1 = justo detrás.
    rear: f32,
    /// Alfa de la sombra de la cabeza por oído (izq., der.).
    alpha: [f32; 2],
    /// Retardo extra por oído (el lejano se retrasa).
    delay: [f32; 2],
    /// Ganancia por oído (algo de ILD también en graves).
    gain: [f32; 2],
}

impl Params {
    fn for_direction(d: Direction) -> Self {
        let x = d.x.clamp(-1.0, 1.0);
        // Coordenadas interaurales: ángulo lateral (-90 izquierda .. 90 derecha) y polar
        // alrededor del eje de los oídos (0 delante, 90 arriba, +-180 detrás).
        let lateral = x.asin();
        let polar = d.y.atan2(d.z).to_degrees();

        let scale = FS / 44_100.0;
        let mut pinna = [0f32; 5];
        for k in 0..5 {
            let tau = PINNA_A[k] * (lateral / 2.0).cos() * (PINNA_D * (90.0 - polar)).to_radians().sin() + PINNA_B[k];
            pinna[k] = (tau * scale).max(1.0);
        }

        let rear = (-d.z).max(0.0) * (1.0 - d.y.abs()).max(0.0);

        // Ángulo respecto al eje de cada oído (0 = en ese oído).
        let theta = |cos: f32| cos.clamp(-1.0, 1.0).acos().to_degrees();
        let alpha_of = |t: f32| (1.0 + ALPHA_MIN / 2.0) + (1.0 - ALPHA_MIN / 2.0) * (t / THETA_MIN * 180.0).to_radians().cos();
        // El modelo da hasta +6 dB de agudos en el oído cercano; con +3.5 dB alguien
        // al lado no suena chillón.
        let alpha = [alpha_of(theta(-x)).min(ALPHA_MAX), alpha_of(theta(x)).min(ALPHA_MAX)];

        // Woodworth: ITD = (a/c)(beta + sin beta). Se retrasa el oído lejano.
        let itd = HEAD_SAMPLES * (lateral.abs() + lateral.abs().sin());
        let delay = if x >= 0.0 { [itd, 0.0] } else { [0.0, itd] };

        // Algo de ILD en todo el espectro, con potencia constante.
        let ild = 0.35 * x;
        let (l, r) = (1.0 - ild, 1.0 + ild);
        let norm = (2.0 / (l * l + r * r)).sqrt();
        Self { pinna, rear, alpha, delay, gain: [l * norm, r * norm] }
    }
}

/// Estado por hablante.
pub struct Binaural {
    /// Entrada mono reciente (ecos de la pinna).
    mono: [f32; DELAY_LEN],
    /// Después de pinna y sombra trasera, antes de separar oídos (para el ITD).
    shaped: [f32; DELAY_LEN],
    pos: usize,
    rear_lp: f32,
    /// Estado de la sombra de la cabeza por oído: (x anterior, y anterior).
    shadow: [(f32, f32); 2],
    current: Option<Params>,
}

impl Default for Binaural {
    fn default() -> Self {
        Self::new()
    }
}

impl Binaural {
    pub fn new() -> Self {
        Self { mono: [0.0; DELAY_LEN], shaped: [0.0; DELAY_LEN], pos: 0, rear_lp: 0.0, shadow: [(0.0, 0.0); 2], current: None }
    }

    fn read(buf: &[f32; DELAY_LEN], pos: usize, delay: f32) -> f32 {
        let d = delay.clamp(0.0, (DELAY_LEN - 2) as f32);
        let i = d.floor();
        let frac = d - i;
        let a = buf[(pos + DELAY_LEN - i as usize) % DELAY_LEN];
        let b = buf[(pos + DELAY_LEN - i as usize - 1) % DELAY_LEN];
        a + (b - a) * frac
    }

    /// Suma `input` (mono) a `out` (estéreo intercalado) desde `dir`, por `gain`.
    /// Los cambios de dirección se interpolan a lo largo de la trama (sin clics).
    pub fn process_add(&mut self, input: &[f32], dir: Direction, gain: f32, out: &mut [f32]) {
        let target = Params::for_direction(dir);
        let start = self.current.unwrap_or(target);
        self.current = Some(target);
        let n = input.len().min(out.len() / 2);
        if n == 0 {
            return;
        }
        let rear_a = 1.0 - (-2.0 * std::f32::consts::PI * REAR_CUTOFF / FS).exp();
        let k2 = 2.0 * FS;
        let t = HEAD_RADIUS / (2.0 * SPEED_OF_SOUND); // 1/(2*w0), w0 = c/a
        let lerp = |a: f32, b: f32, f: f32| a + (b - a) * f;
        for (i, &x) in input.iter().take(n).enumerate() {
            let f = (i + 1) as f32 / n as f32;
            self.pos = (self.pos + 1) % DELAY_LEN;
            self.mono[self.pos] = x;

            // 1) Pinna.
            let mut y = x;
            for ((rho, from), to) in PINNA_RHO.iter().zip(start.pinna).zip(target.pinna) {
                y += PINNA_MIX * rho * Self::read(&self.mono, self.pos, lerp(from, to, f));
            }
            // 2) Sombra trasera.
            self.rear_lp += rear_a * (y - self.rear_lp);
            let rear = lerp(start.rear, target.rear, f);
            y -= REAR_SHADE * rear * (y - self.rear_lp);
            self.shaped[self.pos] = y;

            // 3) y 4) Por oído: ITD y sombra de la cabeza.
            for ear in 0..2 {
                let s = Self::read(&self.shaped, self.pos, lerp(start.delay[ear], target.delay[ear], f));
                let alpha = lerp(start.alpha[ear], target.alpha[ear], f);
                // H(s) = (1 + a*T*s) / (1 + T*s), transformación bilineal.
                let den = 1.0 + t * k2;
                let b0 = (1.0 + alpha * t * k2) / den;
                let b1 = (1.0 - alpha * t * k2) / den;
                let a1 = (1.0 - t * k2) / den;
                let (px, py) = self.shadow[ear];
                let v = b0 * s + b1 * px - a1 * py;
                self.shadow[ear] = (s, v);
                let g = lerp(start.gain[ear], target.gain[ear], f);
                out[2 * i + ear] += v * g * gain * MAKEUP;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ruido blanco.
    fn noise(n: usize) -> Vec<f32> {
        let mut x = 12345u32;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * 0.3
            })
            .collect()
    }

    /// Ruido con espectro aproximado de voz.
    fn speechy(n: usize) -> Vec<f32> {
        let a = 1.0 - (-2.0 * std::f32::consts::PI * 1_500.0 / FS).exp();
        let mut y = 0.0;
        noise(n)
            .into_iter()
            .map(|s| {
                y += a * (s - y);
                y * 2.0
            })
            .collect()
    }

    fn render(dir: Direction) -> (Vec<f32>, Vec<f32>) {
        render_input(dir, &noise(48_000))
    }

    fn render_input(dir: Direction, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut b = Binaural::new();
        let input = input.to_vec();
        let mut out = vec![0f32; input.len() * 2];
        for (chunk, o) in input.chunks(960).zip(out.chunks_mut(1920)) {
            b.process_add(chunk, dir, 1.0, o);
        }
        let l = out.iter().step_by(2).copied().collect();
        let r = out.iter().skip(1).step_by(2).copied().collect();
        (l, r)
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32
    }

    /// Energía en una banda (paso banda de 2º orden, suficiente para comparar).
    fn band(x: &[f32], lo: f32, hi: f32) -> f32 {
        let hp = |x: &[f32], fc: f32| {
            let a = (-2.0 * std::f32::consts::PI * fc / FS).exp();
            let (mut px, mut py) = (0.0, 0.0);
            x.iter()
                .map(|&s| {
                    let y = a * (py + s - px);
                    px = s;
                    py = y;
                    y
                })
                .collect::<Vec<_>>()
        };
        let lp = |x: &[f32], fc: f32| {
            let a = 1.0 - (-2.0 * std::f32::consts::PI * fc / FS).exp();
            let mut y = 0.0;
            x.iter()
                .map(|&s| {
                    y += a * (s - y);
                    y
                })
                .collect::<Vec<_>>()
        };
        energy(&lp(&lp(&hp(&hp(x, lo), lo), hi), hi))
    }

    fn db(a: f32, b: f32) -> f32 {
        10.0 * (a / b).log10()
    }

    fn dir(az_deg: f32, el_deg: f32) -> Direction {
        let (sa, ca) = az_deg.to_radians().sin_cos();
        let (se, ce) = el_deg.to_radians().sin_cos();
        Direction { x: sa * ce, y: se, z: ca * ce }
    }

    #[test]
    fn right_side_is_louder_and_earlier_on_the_right() {
        let (l, r) = render(dir(90.0, 0.0));
        assert!(db(energy(&r), energy(&l)) > 6.0, "ILD {:.1} dB", db(energy(&r), energy(&l)));
        // ITD: la correlación cruzada da el retardo del oído izquierdo (~0.65 ms, ~31 muestras).
        let best = (0..48)
            .max_by(|&a, &b| {
                let c = |lag: usize| (lag..l.len()).map(|i| l[i] * r[i - lag]).sum::<f32>();
                c(a).partial_cmp(&c(b)).unwrap()
            })
            .unwrap();
        assert!((26..=36).contains(&best), "ITD de {best} muestras");
    }

    #[test]
    fn front_and_back_sound_different() {
        // Mismo ángulo lateral delante (30) y detrás (150): mismo ITD, distinto timbre.
        let (fl, fr) = render(dir(30.0, 0.0));
        let (bl, br) = render(dir(150.0, 0.0));
        let hf_front = band(&fl, 4_000.0, 12_000.0) + band(&fr, 4_000.0, 12_000.0);
        let hf_back = band(&bl, 4_000.0, 12_000.0) + band(&br, 4_000.0, 12_000.0);
        let diff = db(hf_front, hf_back);
        eprintln!("agudos delante − detrás: {diff:.1} dB");
        assert!(diff > 3.0, "solo {diff:.1} dB");
    }

    /// Respuesta (dB, ambos oídos) a un tono desde una dirección.
    fn response(d: Direction, freq: f32) -> f32 {
        let tone: Vec<f32> = (0..9_600).map(|i| 0.3 * (2.0 * std::f32::consts::PI * freq * i as f32 / FS).sin()).collect();
        let (l, r) = render_input(d, &tone);
        // Sin el arranque de los filtros.
        10.0 * (energy(&l[960..]) + energy(&r[960..])).log10()
    }

    #[test]
    fn elevation_moves_the_pinna_notch() {
        // De frente a distintas alturas: la muesca cambia de frecuencia.
        let profile = |d| (0..13).map(|k| response(d, 5_000.0 + 500.0 * k as f32)).collect::<Vec<_>>();
        let notch = |p: &[f32]| 5_000.0 + 500.0 * p.iter().enumerate().min_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0 as f32;
        let diff = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
        let (up, level, down) = (profile(dir(0.0, 60.0)), profile(dir(0.0, 0.0)), profile(dir(0.0, -50.0)));
        eprintln!(
            "muesca: arriba {} Hz, al frente {} Hz, abajo {} Hz; diferencias {:.1} / {:.1} / {:.1} dB",
            notch(&up), notch(&level), notch(&down), diff(&up, &level), diff(&down, &level), diff(&up, &down)
        );
        assert!(diff(&up, &level) > 6.0 && diff(&down, &level) > 6.0 && diff(&up, &down) > 6.0);
        assert!(notch(&up) != notch(&level) && notch(&down) != notch(&level));
    }

    #[test]
    fn loudness_is_similar_to_simple_panning() {
        let voice = speechy(48_000);
        let input = energy(&voice);
        for d in [dir(0.0, 0.0), dir(90.0, 0.0), dir(180.0, 0.0), dir(0.0, 70.0)] {
            let (l, r) = render_input(d, &voice);
            let total = db(energy(&l) + energy(&r), 2.0 * input);
            assert!(total.abs() < 3.5, "{d:?}: {total:.1} dB");
        }
    }

    #[test]
    fn turning_does_not_click() {
        // Peor caso: saltar de izquierda a derecha cada trama. Sin saltos bruscos.
        let mut b = Binaural::new();
        let tone: Vec<f32> = (0..48_000).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect();
        let mut out = vec![0f32; tone.len() * 2];
        for (k, (chunk, o)) in tone.chunks(960).zip(out.chunks_mut(1920)).enumerate() {
            let d = if k % 2 == 0 { dir(-90.0, 0.0) } else { dir(90.0, 0.0) };
            b.process_add(chunk, d, 1.0, o);
        }
        let max_step = out.chunks(2).collect::<Vec<_>>().windows(2).map(|w| (w[1][0] - w[0][0]).abs()).fold(0.0, f32::max);
        assert!(max_step < 0.08, "salto de {max_step}");
    }

    #[test]
    fn head_frame_from_minecraft_angles() {
        // Yaw 0: lo que está al sur queda delante; al este, a la izquierda.
        let front = Direction::from_world([0.0, 0.0, 5.0], 0.0, 0.0).unwrap();
        assert!(front.z > 0.99);
        let east = Direction::from_world([5.0, 0.0, 0.0], 0.0, 0.0).unwrap();
        assert!(east.x < -0.99);
        // Pitch -90: lo que está encima queda delante.
        let above = Direction::from_world([0.0, 5.0, 0.0], 0.0, -90.0).unwrap();
        assert!(above.z > 0.99, "{above:?}");
        // Pitch 0: lo de encima está arriba.
        let above = Direction::from_world([0.0, 5.0, 0.0], 0.0, 0.0).unwrap();
        assert!(above.y > 0.99);
        assert!(Direction::from_world([0.0, 0.1, 0.0], 0.0, 0.0).is_none());
    }
}
