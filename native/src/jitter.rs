//! Jitter buffer por hablante.
//!
//! Reordena por número de secuencia y entrega una trama cada 20 ms. Antes de
//! empezar cada frase guarda un colchón que se adapta a la red de quien habla:
//! mide con qué retraso llega cada paquete respecto al más puntual de su frase y
//! espera lo justo para cubrir casi todos. Con buena red, 40 ms; con Wi-Fi o datos
//! móviles, lo que haga falta hasta 160 ms.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Duración de una trama.
const FRAME_MS: f32 = 20.0;
/// Colchón mientras aún no hay medidas (3 x 20 ms = 60 ms).
pub const START_TARGET: usize = 3;
/// Colchón mínimo: 40 ms. Con una sola trama, cualquier paquete un poco tarde
/// respecto al reloj de la tarjeta de sonido ya cortaría.
pub const MIN_TARGET: usize = 2;
/// Colchón máximo: 160 ms. Más retraso ya molesta en una conversación.
pub const MAX_TARGET: usize = 8;
/// Si se acumula más (tras un tirón de red), se salta hacia delante para no
/// arrastrar latencia.
pub const MAX_DEPTH: usize = 12;
/// Tramas vacías seguidas que cierran una frase.
const END_OF_TALK: u32 = 10;
/// Retrasos que se recuerdan: unos 10 s de voz.
const WINDOW: usize = 500;
/// Medidas necesarias antes de fiarse de ellas (1 s de voz).
const MIN_SAMPLES: usize = 50;
/// Parte de los paquetes que el colchón debe cubrir.
const COVERAGE: f32 = 0.97;
/// Un hueco así sin paquetes es una frase nueva: el emisor no envía en silencio.
const PHRASE_GAP: Duration = Duration::from_millis(300);

#[derive(Debug, PartialEq)]
pub enum Pop {
    /// Siguiente trama en orden.
    Frame(Vec<u8>),
    /// Falta la trama esperada pero hay posteriores: ocultar la pérdida. Incluye la
    /// siguiente si está (para FEC).
    Lost { next: Option<Vec<u8>> },
    /// Nada que reproducir.
    Idle,
}

/// Mide con qué retraso llegan los paquetes de un hablante.
///
/// Dentro de una frase el emisor manda una trama cada 20 ms, así que la trama
/// `seq` debería llegar 20 ms después de la `seq - 1`. El retraso de cada una es
/// cuánto más tarde llega que la más puntual de la frase.
#[derive(Default)]
struct DelayMeter {
    /// Primer paquete de la frase: (llegada, secuencia).
    origin: Option<(Instant, u32)>,
    /// Desfase más pequeño visto en la frase (ms): el paquete más puntual.
    earliest: f32,
    last_arrival: Option<Instant>,
    delays: VecDeque<f32>,
}

impl DelayMeter {
    fn record(&mut self, seq: u32, now: Instant) {
        let new_phrase = match (self.origin, self.last_arrival) {
            (Some((_, origin_seq)), Some(last)) => {
                now.duration_since(last) > PHRASE_GAP || seq.wrapping_sub(origin_seq).wrapping_add(1000) > 100_000
            }
            _ => true,
        };
        self.last_arrival = Some(now);
        if new_phrase {
            self.origin = Some((now, seq));
            self.earliest = 0.0;
            return;
        }
        let (origin_time, origin_seq) = self.origin.unwrap_or((now, seq));
        let frames = seq.wrapping_sub(origin_seq) as i32 as f32;
        let offset = now.duration_since(origin_time).as_secs_f32() * 1000.0 - frames * FRAME_MS;
        self.earliest = self.earliest.min(offset);
        if self.delays.len() == WINDOW {
            self.delays.pop_front();
        }
        self.delays.push_back(offset - self.earliest);
    }

    /// Tramas de colchón para cubrir el retraso de casi todos los paquetes, más una
    /// de margen por el desfase con el reloj de la tarjeta de sonido.
    fn target(&self) -> usize {
        if self.delays.len() < MIN_SAMPLES {
            return START_TARGET;
        }
        let mut sorted: Vec<f32> = self.delays.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let index = ((sorted.len() as f32 * COVERAGE) as usize).min(sorted.len() - 1);
        let frames = (sorted[index].max(0.0) / FRAME_MS).ceil() as usize + 1;
        frames.clamp(MIN_TARGET, MAX_TARGET)
    }
}

pub struct JitterBuffer {
    frames: HashMap<u32, Vec<u8>>,
    next: Option<u32>,
    playing: bool,
    empty_run: u32,
    meter: DelayMeter,
    /// Colchón de la frase en curso.
    target: usize,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self {
            frames: HashMap::new(),
            next: None,
            playing: false,
            empty_run: 0,
            meter: DelayMeter::default(),
            target: START_TARGET,
        }
    }
}

/// `a` va antes que `b`, con desbordamiento de u32.
fn before(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Colchón en tramas de 20 ms: el de la frase en curso o, si no habla, el que
    /// usará la siguiente.
    pub fn target(&self) -> usize {
        if self.playing {
            self.target
        } else {
            self.meter.target()
        }
    }

    pub fn push(&mut self, seq: u32, frame: Vec<u8>) {
        self.push_at(seq, frame, Instant::now());
    }

    /// Como [`push`](Self::push), con la hora de llegada (para los tests).
    pub fn push_at(&mut self, seq: u32, frame: Vec<u8>, now: Instant) {
        // Se mide también lo que llega tarde: es justo lo que hay que cubrir.
        self.meter.record(seq, now);
        if self.playing {
            if let Some(next) = self.next {
                if before(seq, next) {
                    return; // llega tarde
                }
            }
        }
        // Sin reproducir se acepta cualquier secuencia (el emisor puede haber reconectado).
        self.frames.entry(seq).or_insert(frame);

        if self.frames.len() > MAX_DEPTH {
            // Saltar a la más reciente menos el colchón.
            let mut seqs: Vec<u32> = self.frames.keys().copied().collect();
            seqs.sort_by(|a, b| if before(*a, *b) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
            let keep = self.target.clamp(1, seqs.len());
            let keep_from = seqs[seqs.len() - keep];
            self.frames.retain(|s, _| !before(*s, keep_from));
            self.next = Some(keep_from);
        }
    }

    fn oldest(&self) -> Option<u32> {
        self.frames.keys().copied().reduce(|a, b| if before(b, a) { b } else { a })
    }

    pub fn pop(&mut self) -> Pop {
        if !self.playing {
            // El colchón se fija al empezar cada frase: cambiarlo a media frase
            // obligaría a acelerar o frenar la voz.
            self.target = self.meter.target();
            if self.frames.len() < self.target {
                return Pop::Idle;
            }
            self.playing = true;
            self.empty_run = 0;
            self.next = self.oldest();
        }

        let next = match self.next {
            Some(n) => n,
            None => return Pop::Idle,
        };

        if let Some(frame) = self.frames.remove(&next) {
            self.next = Some(next.wrapping_add(1));
            self.empty_run = 0;
            return Pop::Frame(frame);
        }

        if self.frames.is_empty() {
            // Nada en el búfer: o ha dejado de hablar o su siguiente paquete viene
            // tarde. Se espera por él sin avanzar, así si llega se oye (un poco más
            // tarde) en vez de tirarlo; si no llega, la frase se cierra.
            self.empty_run += 1;
            if self.empty_run >= END_OF_TALK {
                self.playing = false;
                return Pop::Idle;
            }
            // Unas pocas tramas de PLC para que el corte no sea seco.
            return if self.empty_run <= 2 { Pop::Lost { next: None } } else { Pop::Idle };
        }

        // Hueco en medio: trama perdida.
        self.next = Some(next.wrapping_add(1));
        Pop::Lost { next: self.frames.get(&next.wrapping_add(1)).cloned() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn f(n: u8) -> Vec<u8> {
        vec![n]
    }

    #[test]
    fn waits_for_target_depth() {
        let mut jb = JitterBuffer::new();
        jb.push(0, f(0));
        jb.push(1, f(1));
        assert_eq!(jb.pop(), Pop::Idle);
        jb.push(2, f(2));
        assert_eq!(jb.pop(), Pop::Frame(f(0)));
    }

    #[test]
    fn reorders() {
        let mut jb = JitterBuffer::new();
        for s in [2, 0, 1, 3] {
            jb.push(s, f(s as u8));
        }
        for s in 0..4 {
            assert_eq!(jb.pop(), Pop::Frame(f(s)));
        }
    }

    #[test]
    fn detects_loss_and_offers_fec_source() {
        let mut jb = JitterBuffer::new();
        for s in [10, 12, 13] {
            jb.push(s, f(s as u8));
        }
        assert_eq!(jb.pop(), Pop::Frame(f(10)));
        assert_eq!(jb.pop(), Pop::Lost { next: Some(f(12)) });
        assert_eq!(jb.pop(), Pop::Frame(f(12)));
        assert_eq!(jb.pop(), Pop::Frame(f(13)));
    }

    #[test]
    fn drops_late_and_duplicate_packets() {
        let mut jb = JitterBuffer::new();
        for s in 0..3 {
            jb.push(s, f(s as u8));
        }
        assert_eq!(jb.pop(), Pop::Frame(f(0)));
        jb.push(0, f(99)); // tarde
        jb.push(1, f(99)); // duplicado
        assert_eq!(jb.pop(), Pop::Frame(f(1)));
        assert_eq!(jb.depth(), 1);
    }

    #[test]
    fn survives_sequence_wraparound() {
        let mut jb = JitterBuffer::new();
        let start = u32::MAX - 1;
        for i in 0..4u32 {
            jb.push(start.wrapping_add(i), f(i as u8));
        }
        for i in 0..4u8 {
            assert_eq!(jb.pop(), Pop::Frame(f(i)));
        }
    }

    #[test]
    fn stops_after_talk_ends_and_restarts() {
        let mut jb = JitterBuffer::new();
        for s in 0..3 {
            jb.push(s, f(0));
        }
        for _ in 0..3 {
            jb.pop();
        }
        for _ in 0..END_OF_TALK {
            jb.pop();
        }
        assert!(!jb.is_playing());
        // Nueva frase con secuencia muy posterior.
        for s in 500..503 {
            jb.push(s, f(1));
        }
        assert_eq!(jb.pop(), Pop::Frame(f(1)));
    }

    #[test]
    fn plays_a_late_packet_instead_of_dropping_it() {
        let mut jb = JitterBuffer::new();
        for s in 0..3 {
            jb.push(s, f(s as u8));
        }
        for s in 0..3 {
            assert_eq!(jb.pop(), Pop::Frame(f(s)));
        }
        // El 3 viene tarde: se oculta el hueco y, cuando llega, se oye.
        assert_eq!(jb.pop(), Pop::Lost { next: None });
        jb.push(3, f(3));
        assert_eq!(jb.pop(), Pop::Frame(f(3)));
    }

    /// Red simulada: una trama cada 20 ms, cada una con un retraso de red de
    /// `base` ms más uno al azar entre 0 y `jitter` ms. La tarjeta de sonido pide
    /// una trama cada 20 ms. Frases de 5 s separadas por 1 s de silencio.
    struct Net {
        rng: u64,
        base: u32,
        jitter: u32,
    }

    impl Net {
        fn delay(&mut self) -> u32 {
            self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.base + ((self.rng >> 33) as u32) % (self.jitter + 1)
        }
    }

    /// Resultado de una frase: tramas que no sonaron a tiempo (cortes) y retraso
    /// medio desde que se envió cada trama hasta que sonó, sin contar la red.
    struct Phrase {
        cuts: usize,
        delay_ms: f32,
    }

    fn talk(jb: &mut JitterBuffer, net: &mut Net, seq0: u32, start: Instant) -> Phrase {
        const FRAMES: u32 = 250;
        let mut arrivals: Vec<(u32, u32)> = (0..FRAMES).map(|i| (i * 20 + net.delay(), i)).collect();
        arrivals.sort();
        let mut next_arrival = 0;
        let (mut cuts, mut delay_sum, mut played) = (0usize, 0f32, 0u32);
        let mut started = false;
        // La tarjeta de sonido va desfasada 7 ms respecto al emisor.
        let mut tick = 7;
        while played < FRAMES && tick < FRAMES * 20 + 2000 {
            while next_arrival < arrivals.len() && arrivals[next_arrival].0 <= tick {
                let (at, i) = arrivals[next_arrival];
                jb.push_at(seq0 + i, i.to_le_bytes().to_vec(), start + Duration::from_millis(at as u64));
                next_arrival += 1;
            }
            match jb.pop() {
                Pop::Frame(v) => {
                    let i = u32::from_le_bytes(v[..4].try_into().unwrap());
                    started = true;
                    played = i + 1;
                    delay_sum += (tick - i * 20) as f32 - net.base as f32;
                }
                Pop::Lost { .. } => {
                    played += 1;
                    cuts += 1;
                }
                Pop::Idle if started => cuts += 1,
                Pop::Idle => {}
            }
            tick += 20;
        }
        // Cierre de la frase (silencio).
        for _ in 0..END_OF_TALK {
            jb.pop();
        }
        Phrase { cuts, delay_ms: delay_sum / FRAMES as f32 }
    }

    /// Varias frases seguidas; devuelve la última.
    fn conversation(jb: &mut JitterBuffer, net: &mut Net, phrases: u32, t0: Instant, seq0: &mut u32) -> Phrase {
        let mut last = None;
        for k in 0..phrases {
            let start = t0 + Duration::from_millis(k as u64 * 6000);
            last = Some(talk(jb, net, *seq0, start));
            *seq0 += 250;
        }
        last.unwrap()
    }

    #[test]
    fn good_network_gets_low_delay() {
        let mut jb = JitterBuffer::new();
        let mut net = Net { rng: 1, base: 30, jitter: 3 };
        let p = conversation(&mut jb, &mut net, 3, Instant::now(), &mut 0);
        assert_eq!(jb.target(), MIN_TARGET);
        assert_eq!(p.cuts, 0);
        assert!(p.delay_ms <= 45.0, "retraso {:.1} ms", p.delay_ms);
    }

    #[test]
    fn bad_network_gets_a_bigger_buffer_and_few_cuts() {
        let mut jb = JitterBuffer::new();
        // Wi-Fi malo: hasta 80 ms de diferencia entre paquetes.
        let mut net = Net { rng: 7, base: 30, jitter: 80 };
        let p = conversation(&mut jb, &mut net, 3, Instant::now(), &mut 0);
        assert!((4..=MAX_TARGET).contains(&jb.target()), "colchón {}", jb.target());
        assert!(p.cuts <= 2, "{} cortes en 250 tramas", p.cuts);
        assert!(p.delay_ms <= 160.0, "retraso {:.1} ms", p.delay_ms);
    }

    #[test]
    fn delay_goes_back_down_when_the_network_recovers() {
        let mut jb = JitterBuffer::new();
        let t0 = Instant::now();
        let mut seq = 0;
        conversation(&mut jb, &mut Net { rng: 3, base: 30, jitter: 80 }, 3, t0, &mut seq);
        assert!(jb.target() >= 4);
        let p = conversation(&mut jb, &mut Net { rng: 5, base: 30, jitter: 3 }, 4, t0 + Duration::from_secs(60), &mut seq);
        assert_eq!(jb.target(), MIN_TARGET);
        assert_eq!(p.cuts, 0);
    }

    #[test]
    fn skips_ahead_when_too_deep() {
        let mut jb = JitterBuffer::new();
        for s in 0..(MAX_DEPTH as u32 + 5) {
            jb.push(s, f(s as u8));
        }
        assert!(jb.depth() <= MAX_DEPTH);
        let first = match jb.pop() {
            Pop::Frame(v) => v[0] as usize,
            other => panic!("{other:?}"),
        };
        assert!(first >= MAX_DEPTH - START_TARGET, "debería saltar latencia, empezó en {first}");
    }
}
