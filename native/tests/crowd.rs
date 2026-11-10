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

    /// Hablan todos a la vez y el oyente mezcla. Primero espera a que el reparto esté
    /// hecho (`settled`, con el número de voces separadas y si llega el murmullo) y
    /// luego mira 600 ms: a quién oye por separado, si le llega el murmullo y cuánta
    /// energía recibe.
    ///
    /// No vale mirar en un solo instante, porque una trama perdida haría creer que
    /// alguien no está; ni desde que empiezan a hablar, porque hasta que el reparto no
    /// está hecho llegan todas las voces por separado, y cuánto tarda eso depende de la
    /// máquina (aquí ~350 ms, en una cargada bastante más).
    fn talk(&self, settled: impl Fn(usize, bool) -> bool) -> (HashSet<[u8; 16]>, bool, f32) {
        for t in &self.talkers {
            t.set_push_to_talk(true);
        }
        // Solo mezcla el oyente: los que hablan no necesitan oír nada, y mezclar 31
        // clientes en este hilo no llega a tiempo real en una máquina lenta.
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        let start = Instant::now();
        let mut n = 0u32;

        // Hablando, hasta que el reparto esté hecho.
        let mut ready = false;
        while n < 250 {
            self.frame(n, &mut out, start);
            n += 1;
            if settled(self.listener.speaking_players().len(), self.listener.hearing_crowd()) {
                ready = true;
                break;
            }
        }
        let separate = self.listener.speaking_players().len();
        assert!(ready, "el reparto no se hizo en 5 s: {separate} voces separadas");

        // Y ya midiendo.
        let mut energy = 0f32;
        let mut heard: HashSet<[u8; 16]> = HashSet::new();
        let mut crowd = false;
        for _ in 0..30 {
            self.frame(n, &mut out, start);
            n += 1;
            energy += out.iter().map(|s| s * s).sum::<f32>();
            heard.extend(self.listener.speaking_players().into_iter().map(|(id, _)| id));
            crowd |= self.listener.hearing_crowd();
        }

        for t in &self.talkers {
            t.set_push_to_talk(false);
        }
        std::thread::sleep(Duration::from_millis(600));
        (heard, crowd, energy)
    }

    /// Una trama de 20 ms: hablan todos y el oyente mezcla. A ritmo de reloj, así que si
    /// una vuelta se retrasa la siguiente no espera.
    fn frame(&self, n: u32, out: &mut [f32], start: Instant) {
        for (k, t) in self.talkers.iter().enumerate() {
            t.push_capture(&sine_frame(n as usize, 200.0 + 20.0 * k as f32));
        }
        let next = start + Duration::from_millis(20) * (n + 1);
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        self.listener.render(out);
    }
}

#[test]
fn a_crowd_sends_the_nearest_voices_and_mixes_the_rest() {
    let c = Crowd::new(24);
    // Hecho = ya no llegan más de 24 por separado y suena el murmullo.
    let (heard, crowd, energy) = c.talk(|separate, crowd| separate <= 24 && crowd);
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
    // El anfitrión no cuenta para el límite, así que lo estable son 24 y él.
    let (heard, crowd, _) = c.talk(|separate, crowd| separate <= 25 && crowd);
    assert!(crowd, "con 30 voces y límite 24 sigue habiendo murmullo");
    assert!(heard.contains(&id(TALKERS)), "el anfitrión va siempre por separado");
    // El anfitrión no cuenta para el límite: las 24 más cercanas y él.
    assert!(heard.len() <= 25, "como mucho 24 más el anfitrión, llegan {}", heard.len());
}

#[test]
fn without_limit_everyone_goes_separately() {
    let c = Crowd::new(0);
    // Sin límite, hecho = llegan todas y no hay murmullo.
    let (heard, crowd, _) = c.talk(|separate, crowd| separate == TALKERS && !crowd);
    assert!(!crowd, "sin límite no hay murmullo");
    assert_eq!(heard.len(), TALKERS, "llegan todos por separado");
}

#[test]
fn few_voices_do_not_trigger_the_crowd() {
    let c = Crowd::new(40);
    // Con el límite por encima del número de voces, lo estable es que lleguen todas.
    let (heard, crowd, _) = c.talk(|separate, crowd| separate == TALKERS && !crowd);
    assert!(!crowd, "30 voces con límite 40: no hay multitud");
    assert_eq!(heard.len(), TALKERS);
}
