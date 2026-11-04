//! Jitter buffer por hablante.
//!
//! Reordena por número de secuencia y entrega una trama cada 20 ms, con un
//! pequeño colchón antes de empezar.

use std::collections::HashMap;

/// Colchón inicial (3 x 20 ms = 60 ms).
pub const TARGET_DEPTH: usize = 3;
/// Si se acumula más (tras un tirón de red), se salta hacia delante para no
/// arrastrar latencia.
pub const MAX_DEPTH: usize = 12;
/// Tramas vacías seguidas que cierran una frase.
const END_OF_TALK: u32 = 10;

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

#[derive(Default)]
pub struct JitterBuffer {
    frames: HashMap<u32, Vec<u8>>,
    next: Option<u32>,
    playing: bool,
    empty_run: u32,
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

    pub fn push(&mut self, seq: u32, frame: Vec<u8>) {
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
            let keep_from = seqs[seqs.len() - TARGET_DEPTH];
            self.frames.retain(|s, _| !before(*s, keep_from));
            self.next = Some(keep_from);
        }
    }

    fn oldest(&self) -> Option<u32> {
        self.frames.keys().copied().reduce(|a, b| if before(b, a) { b } else { a })
    }

    pub fn pop(&mut self) -> Pop {
        if !self.playing {
            if self.frames.len() < TARGET_DEPTH {
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
            // Underflow: probablemente ha dejado de hablar.
            self.empty_run += 1;
            self.next = Some(next.wrapping_add(1));
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
        assert!(first >= MAX_DEPTH - TARGET_DEPTH, "debería saltar latencia, empezó en {first}");
    }
}
