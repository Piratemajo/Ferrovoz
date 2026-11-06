//! Multitudes de extremo a extremo: servidor real por UDP en localhost y 31
//! clientes. Con más voces al alcance que `max_voices`, cada oyente recibe por
//! separado las más cercanas y el resto en el murmullo.

use ferrovoz::client::VoiceClient;
use ferrovoz::protocol::{FRAME_SAMPLES, SAMPLE_RATE};
use ferrovoz::server::VoiceServer;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const OVERWORLD: i32 = 1;
const TALKERS: usize = 30;

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(10), "timeout esperando: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sine_frame(n: usize, hz: f32) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|i| 0.4 * (std::f32::consts::TAU * hz * (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

fn id(n: usize) -> [u8; 16] {
    [n as u8 + 1; 16]
}

/// Un oyente en el origen y 30 hablando en fila hacia el este, a 1..30 bloques.
struct Crowd {
    server: VoiceServer,
    listener: VoiceClient,
    talkers: Vec<VoiceClient>,
}

impl Crowd {
    fn new(max_voices: usize) -> Self {
        let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
        server.set_max_voices(max_voices);
        let addr = server.local_addr().unwrap();
        let join = |n: usize, x: f64| {
            let key = server.add_player(id(n));
            server.update_player(&id(n), [x, 64.0, 0.5], OVERWORLD);
            let c = VoiceClient::start(addr, id(n), key, false).unwrap();
            c.set_noise_suppression(false);
            c
        };
        let listener = join(0, 0.5);
        let talkers: Vec<VoiceClient> = (1..=TALKERS).map(|n| join(n, n as f64 + 0.5)).collect();
        wait_until("conectados", || listener.is_connected() && talkers.iter().all(|c| c.is_connected()));
        Self { server, listener, talkers }
    }

    /// Hablan todos 1,5 s. Devuelve a quién oye por separado el oyente al final, si
    /// le llega el murmullo y la energía que recibe.
    fn talk(&self) -> (HashSet<[u8; 16]>, bool, f32) {
        for t in &self.talkers {
            t.set_push_to_talk(true);
        }
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        let mut energy = 0f32;
        for n in 0..75 {
            for (k, t) in self.talkers.iter().enumerate() {
                t.push_capture(&sine_frame(n, 200.0 + 20.0 * k as f32));
            }
            std::thread::sleep(Duration::from_millis(20));
            self.listener.render(&mut out);
            if n > 40 {
                energy += out.iter().map(|s| s * s).sum::<f32>();
            }
            for t in &self.talkers {
                t.render(&mut out);
            }
        }
        let heard: HashSet<[u8; 16]> = self.listener.speaking_players().into_iter().map(|(id, _)| id).collect();
        let crowd = self.listener.hearing_crowd();
        for t in &self.talkers {
            t.set_push_to_talk(false);
        }
        std::thread::sleep(Duration::from_millis(600));
        (heard, crowd, energy)
    }
}

#[test]
fn a_crowd_sends_the_nearest_voices_and_mixes_the_rest() {
    let c = Crowd::new(24);
    let (heard, crowd, energy) = c.talk();
    assert!(crowd, "con 30 voces y límite 24 debe llegar el murmullo");
    assert!(energy > 1.0, "se oye algo");
    assert!(heard.len() <= 24, "como mucho 24 por separado, llegan {}", heard.len());
    for n in 1..=20 {
        assert!(heard.contains(&id(n)), "el de {n} bloques está cerca y debe ir por separado");
    }
    for n in 27..=TALKERS {
        assert!(!heard.contains(&id(n)), "el de {n} bloques debe ir al murmullo");
    }
}

#[test]
fn the_event_host_is_always_heard_separately() {
    let c = Crowd::new(24);
    // El más lejano es el anfitrión.
    c.server.set_priority(&id(TALKERS), true);
    let (heard, crowd, _) = c.talk();
    assert!(crowd);
    assert!(heard.contains(&id(TALKERS)), "el anfitrión va siempre por separado");
}

#[test]
fn without_limit_everyone_goes_separately() {
    let c = Crowd::new(0);
    let (heard, crowd, _) = c.talk();
    assert!(!crowd, "sin límite no hay murmullo");
    assert_eq!(heard.len(), TALKERS, "llegan todos por separado");
}

#[test]
fn few_voices_do_not_trigger_the_crowd() {
    let c = Crowd::new(40);
    let (heard, crowd, _) = c.talk();
    assert!(!crowd, "30 voces con límite 40: no hay multitud");
    assert_eq!(heard.len(), TALKERS);
}
