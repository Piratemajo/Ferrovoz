//! Multitudes.
//!
//! Cuando a una zona le llegan más voces de las que se pueden distinguir
//! (`max_voices`), sus oyentes reciben por separado solo las más cercanas, con su
//! audio 3D, paredes y eco. Las demás no se pierden: el servidor las mezcla en un
//! solo canal, el murmullo de la multitud, que suena desde donde está el grueso de
//! la gente.
//!
//! Las zonas son cubos de [`CELL`] bloques. Todos los oyentes de una zona comparten
//! qué voces van por separado y la misma mezcla, así que el coste depende del
//! número de zonas con gente, no del de oyentes: con 250 personas juntas, unas
//! decenas de mezclas cada 20 ms.
//!
//! El murmullo llega como un hablante más ([`crate::protocol::CROWD_ID`]), así que los clientes no
//! necesitan saber nada de esto.

use crate::codec::{VoiceDecoder, VoiceEncoder};
use crate::protocol::{PlayerId, FRAME_SAMPLES};
use crate::spatial::{distance_gain, SpatialParams};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Lado de una zona, en bloques.
pub const CELL: f64 = 8.0;
/// Del centro de una zona a su esquina más lejana.
const HALF_DIAGONAL: f64 = CELL * 0.866;
/// Calidad del murmullo: menos que una voz (32 kbps), sobra para un fondo.
pub const CROWD_BITRATE: i32 = 24_000;
/// Complejidad de Opus para el murmullo: unas 3 veces más rápido que la de serie,
/// con una diferencia que no se oye en un fondo de voces.
const CROWD_COMPLEXITY: i32 = 3;
/// Volumen del murmullo respecto a una voz a volumen completo.
const CROWD_LEVEL: f32 = 0.7;
/// Tramas en cola por hablante (colchón contra el jitter de su red).
const QUEUE: usize = 4;
/// Sin tramas en este tiempo, el hablante sale de la mezcla.
const VOICE_TTL: Duration = Duration::from_secs(1);
/// Códecs sin usar este tiempo se liberan.
const CODEC_TTL: Duration = Duration::from_secs(5);

/// Zona: cubo de [`CELL`] bloques de una dimensión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    pub dim: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Cell {
    pub fn of(pos: [f64; 3], dim: i32) -> Self {
        let c = |v: f64| (v / CELL).floor() as i32;
        Self { dim, x: c(pos[0]), y: c(pos[1]), z: c(pos[2]) }
    }

    pub fn center(&self) -> [f64; 3] {
        let c = |v: i32| (v as f64 + 0.5) * CELL;
        [c(self.x), c(self.y), c(self.z)]
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Alguien que está hablando.
#[derive(Debug, Clone, Copy)]
pub struct Speaker {
    pub id: PlayerId,
    pub pos: [f64; 3],
    pub dim: i32,
    pub range: f32,
}

/// Para cada zona con más de `max` voces al alcance, las `max` más cercanas a su
/// centro (las que van por separado). Las zonas que no aparecen no tienen límite.
pub fn plan(cells: impl IntoIterator<Item = Cell>, speakers: &[Speaker], max: usize) -> HashMap<Cell, HashSet<PlayerId>> {
    let mut plans = HashMap::new();
    if max == 0 {
        return plans;
    }
    let mut near: Vec<(f64, PlayerId)> = Vec::new();
    for cell in cells {
        let center = cell.center();
        near.clear();
        for s in speakers {
            if s.dim != cell.dim {
                continue;
            }
            let d = distance(center, s.pos);
            // Al alcance de algún punto de la zona.
            if d <= s.range as f64 + HALF_DIAGONAL {
                near.push((d, s.id));
            }
        }
        if near.len() > max {
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            plans.insert(cell, near[..max].iter().map(|(_, id)| *id).collect());
        }
    }
    plans
}

struct Pending {
    frames: VecDeque<Vec<u8>>,
    speaker: Speaker,
    last: Instant,
}

/// Tramas de las voces que van al murmullo de alguna zona, hasta que se mezclan.
#[derive(Default)]
pub struct Stash {
    voices: HashMap<PlayerId, Pending>,
}

impl Stash {
    pub fn push(&mut self, speaker: Speaker, opus: Vec<u8>, now: Instant) {
        let p = self.voices.entry(speaker.id).or_insert_with(|| Pending {
            frames: VecDeque::new(),
            speaker,
            last: now,
        });
        if p.frames.len() >= QUEUE {
            p.frames.pop_front(); // mejor perder lo viejo que acumular retraso
        }
        p.frames.push_back(opus);
        p.speaker = speaker;
        p.last = now;
    }

    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    /// Una trama de cada voz para esta mezcla. Olvida las que ya no hablan.
    pub fn take(&mut self, now: Instant) -> Vec<(Speaker, Vec<u8>)> {
        self.voices.retain(|_, p| now.duration_since(p.last) < VOICE_TTL);
        self.voices
            .values_mut()
            .filter_map(|p| p.frames.pop_front().map(|f| (p.speaker, f)))
            .collect()
    }
}

/// Murmullo de una zona listo para enviar.
pub struct CellMix {
    pub opus: Vec<u8>,
    /// De dónde viene: centro de las voces mezcladas, pesado por su volumen.
    pub centroid: [f64; 3],
}

/// Voz decodificada, lista para mezclar.
pub struct Decoded {
    pub speaker: Speaker,
    pub pcm: Box<[f32; FRAME_SAMPLES]>,
}

/// Decodifica y mezcla. Vive en el hilo de multitudes, fuera del lock del relé.
#[derive(Default)]
pub struct Mixer {
    decoders: HashMap<PlayerId, (VoiceDecoder, Instant)>,
    encoders: HashMap<Cell, (VoiceEncoder, Instant)>,
}

impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cada voz se decodifica una vez y sirve para todas las zonas.
    pub fn decode(&mut self, frames: Vec<(Speaker, Vec<u8>)>, now: Instant) -> Vec<Decoded> {
        let mut out = Vec::with_capacity(frames.len());
        for (speaker, opus) in frames {
            let entry = match self.decoders.entry(speaker.id) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => match VoiceDecoder::new() {
                    Ok(d) => e.insert((d, now)),
                    Err(_) => continue,
                },
            };
            entry.1 = now;
            let mut pcm = Box::new([0f32; FRAME_SAMPLES]);
            if entry.0.decode(&opus, &mut pcm) > 0 {
                out.push(Decoded { speaker, pcm });
            }
        }
        out
    }

    /// Murmullo de `cell`: las voces que le llegan y no van por separado (`near`),
    /// cada una con el volumen que tendría en el centro de la zona. `None` si no le
    /// llega ninguna.
    pub fn mix(&mut self, cell: Cell, near: &HashSet<PlayerId>, voices: &[Decoded], now: Instant) -> Option<CellMix> {
        let center = cell.center();
        let mut sum = [0f32; FRAME_SAMPLES];
        let mut energy = 0f32;
        let mut weight = 0f64;
        let mut centroid = [0f64; 3];
        for v in voices {
            let s = &v.speaker;
            if s.dim != cell.dim || near.contains(&s.id) {
                continue;
            }
            let params = SpatialParams {
                max_distance: s.range,
                full_volume_distance: 4f32.min(s.range * 0.25),
                behind_gain: 1.0,
            };
            let g = distance_gain(distance(center, s.pos) as f32, &params);
            if g <= 0.0 {
                continue;
            }
            for (o, x) in sum.iter_mut().zip(v.pcm.iter()) {
                *o += g * x;
            }
            energy += g * g;
            weight += g as f64;
            for (c, p) in centroid.iter_mut().zip(s.pos) {
                *c += g as f64 * p;
            }
        }
        if weight <= 0.0 {
            return None;
        }
        // Voces sin relación entre sí suman energía, no amplitud: se escala por la
        // raíz para que cien voces suenen como un fondo y no como un grito.
        let scale = CROWD_LEVEL / energy.sqrt().max(1.0);
        for o in sum.iter_mut() {
            *o = (*o * scale).tanh();
        }
        for c in centroid.iter_mut() {
            *c /= weight;
        }

        let encoder = match self.encoders.entry(cell) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let mut enc = VoiceEncoder::new(CROWD_BITRATE).ok()?;
                enc.set_complexity(CROWD_COMPLEXITY);
                e.insert((enc, now))
            }
        };
        encoder.1 = now;
        let opus = encoder.0.encode(&sum).ok()?.to_vec();
        Some(CellMix { opus, centroid })
    }

    /// Libera los códecs de quien ya no habla y de las zonas vacías.
    pub fn cleanup(&mut self, now: Instant) {
        self.decoders.retain(|_, (_, t)| now.duration_since(*t) < CODEC_TTL);
        self.encoders.retain(|_, (_, t)| now.duration_since(*t) < CODEC_TTL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u16) -> PlayerId {
        let mut id = [0u8; 16];
        id[..2].copy_from_slice(&n.to_le_bytes());
        id
    }

    fn speaker(n: u16, x: f64) -> Speaker {
        Speaker { id: id(n), pos: [x, 64.0, 4.0], dim: 0, range: 48.0 }
    }

    #[test]
    fn cells_are_cubes_of_eight_blocks() {
        assert_eq!(Cell::of([0.0, 0.0, 0.0], 0), Cell::of([7.9, 7.9, 7.9], 0));
        assert_ne!(Cell::of([7.9, 0.0, 0.0], 0), Cell::of([8.0, 0.0, 0.0], 0));
        assert_eq!(Cell::of([-0.1, 0.0, 0.0], 0).x, -1);
        assert_eq!(Cell::of([3.0, 70.0, 12.0], 0).center(), [4.0, 68.0, 12.0]);
    }

    #[test]
    fn no_plan_while_there_are_few_voices() {
        let speakers: Vec<Speaker> = (0..10).map(|n| speaker(n, n as f64)).collect();
        let cell = Cell::of([0.0, 64.0, 4.0], 0);
        assert!(plan([cell], &speakers, 24).is_empty());
        assert!(plan([cell], &speakers, 0).is_empty(), "0 = sin límite");
    }

    #[test]
    fn crowded_cell_keeps_the_nearest_voices() {
        // 40 hablando en fila; la zona está en x = 0..8.
        let speakers: Vec<Speaker> = (0..40).map(|n| speaker(n, n as f64)).collect();
        let cell = Cell::of([0.0, 64.0, 4.0], 0);
        let plans = plan([cell], &speakers, 24);
        let near = &plans[&cell];
        assert_eq!(near.len(), 24);
        for n in 0..20 {
            assert!(near.contains(&id(n)), "{n} está cerca y debería ir por separado");
        }
        for n in 30..40 {
            assert!(!near.contains(&id(n)), "{n} está lejos y debería ir al murmullo");
        }
    }

    #[test]
    fn other_dimensions_and_out_of_range_do_not_count() {
        let mut speakers: Vec<Speaker> = (0..30).map(|n| Speaker { dim: 1, ..speaker(n, n as f64) }).collect();
        speakers.extend((100..130).map(|n| speaker(n, 500.0 + n as f64)));
        assert!(plan([Cell::of([0.0, 64.0, 4.0], 0)], &speakers, 24).is_empty());
    }

    #[test]
    fn stash_keeps_a_short_queue_and_forgets_silent_voices() {
        let mut stash = Stash::default();
        let t0 = Instant::now();
        for k in 0..10u8 {
            stash.push(speaker(1, 0.0), vec![k], t0);
        }
        let frames = stash.take(t0);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].1, vec![6], "solo las {QUEUE} últimas");
        assert!(stash.take(t0 + VOICE_TTL + Duration::from_millis(1)).is_empty());
        assert!(stash.is_empty());
    }

    /// Voz de prueba: un tono de `hz` Hz.
    fn tone(enc: &mut VoiceEncoder, hz: f32, frame: usize) -> Vec<u8> {
        let pcm: Vec<f32> = (0..FRAME_SAMPLES)
            .map(|i| {
                let t = (frame * FRAME_SAMPLES + i) as f32 / 48_000.0;
                0.3 * (t * hz * std::f32::consts::TAU).sin()
            })
            .collect();
        enc.encode(&pcm).unwrap().to_vec()
    }

    fn rms(pcm: &[f32]) -> f32 {
        (pcm.iter().map(|x| x * x).sum::<f32>() / pcm.len() as f32).sqrt()
    }

    #[test]
    fn mix_excludes_near_voices_and_comes_from_the_crowd() {
        let cell = Cell::of([0.0, 64.0, 4.0], 0);
        let mut mixer = Mixer::new();
        let now = Instant::now();
        let mut encoders: Vec<VoiceEncoder> = (0..3).map(|_| VoiceEncoder::new(32_000).unwrap()).collect();
        // 1 cerca (va por separado) y 2 a 20 bloques al este.
        let speakers = [speaker(1, 2.0), speaker(2, 24.0), speaker(3, 26.0)];
        let near: HashSet<PlayerId> = [id(1)].into();
        let mut last = None;
        for frame in 0..10 {
            let frames: Vec<(Speaker, Vec<u8>)> = speakers
                .iter()
                .zip(encoders.iter_mut())
                .enumerate()
                .map(|(k, (s, e))| (*s, tone(e, 200.0 + 150.0 * k as f32, frame)))
                .collect();
            let voices = mixer.decode(frames, now);
            last = mixer.mix(cell, &near, &voices, now);
        }
        let mix = last.expect("hay voces lejanas");
        assert!((mix.centroid[0] - 25.0).abs() < 1.5, "viene de la multitud: {:?}", mix.centroid);
        let mut dec = VoiceDecoder::new().unwrap();
        let mut out = [0f32; FRAME_SAMPLES];
        assert_eq!(dec.decode(&mix.opus, &mut out), FRAME_SAMPLES);
        assert!(rms(&out) > 0.01, "el murmullo se oye");

        // Si todas van por separado no hay murmullo.
        let all: HashSet<PlayerId> = [id(1), id(2), id(3)].into();
        let voices = mixer.decode(vec![(speakers[1], tone(&mut encoders[1], 350.0, 10))], now);
        assert!(mixer.mix(cell, &all, &voices, now).is_none());
    }

    #[test]
    fn a_hundred_voices_do_not_clip() {
        let cell = Cell::of([0.0, 64.0, 4.0], 0);
        let mut mixer = Mixer::new();
        let now = Instant::now();
        let mut enc = VoiceEncoder::new(32_000).unwrap();
        let speakers: Vec<Speaker> = (0..100).map(|n| speaker(n, 10.0 + (n % 10) as f64)).collect();
        let mut out = [0f32; FRAME_SAMPLES];
        let mut dec = VoiceDecoder::new().unwrap();
        for frame in 0..5 {
            let opus = tone(&mut enc, 300.0, frame);
            let voices = mixer.decode(speakers.iter().map(|s| (*s, opus.clone())).collect(), now);
            let mix = mixer.mix(cell, &HashSet::new(), &voices, now).unwrap();
            dec.decode(&mix.opus, &mut out);
        }
        assert!(out.iter().all(|x| x.abs() <= 1.0));
        assert!(rms(&out) < 0.8, "cien voces no deben saturar: {}", rms(&out));
    }
}
