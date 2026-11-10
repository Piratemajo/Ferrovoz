//! Lo que usa la API de Java para plugins, de extremo a extremo: relé real por UDP
//! en localhost y clientes reales. Canales de audio, alcance propio, silencio
//! general, escucha del micro y eventos.

use ferrovoz::channels::{self, Target};
use ferrovoz::codec::VoiceDecoder;
use ferrovoz::client::VoiceClient;
use ferrovoz::protocol::{TalkMode, FRAME_SAMPLES, SAMPLE_RATE};
use ferrovoz::server::{EventKind, VoiceServer};
use std::collections::HashSet;
use std::time::{Duration, Instant};

const OVERWORLD: i32 = 1;

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(10), "timeout esperando: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Espera a que no llegue nada más a la escucha del micro, y lo tira. Lo que quede en
/// camino de una frase anterior aparecería si no en la medida siguiente: en una máquina
/// lenta, las últimas tramas tardan en salir del cliente.
fn drain_mic(server: &VoiceServer) {
    wait_until("deja de llegar voz al micro", || {
        server.poll_mic();
        std::thread::sleep(Duration::from_millis(100));
        server.poll_mic().is_empty()
    });
}

fn sine_frame(n: usize) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|i| 0.5 * (std::f32::consts::TAU * 300.0 * (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

fn id(n: u8) -> [u8; 16] {
    [n + 40; 16]
}

/// Relé y un cliente por cada `x` (todos en el overworld, en fila).
struct Scene {
    server: VoiceServer,
    clients: Vec<VoiceClient>,
}

impl Scene {
    fn new(xs: &[f64]) -> Self {
        let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
        let addr = server.local_addr().unwrap();
        let clients: Vec<VoiceClient> = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let key = server.add_player(id(i as u8));
                server.update_player(&id(i as u8), [*x, 64.0, 0.0], OVERWORLD);
                let c = VoiceClient::start(addr, id(i as u8), key, false).unwrap();
                c.set_noise_suppression(false);
                c
            })
            .collect();
        wait_until("conectados", || clients.iter().all(|c| c.is_connected()));
        Self { server, clients }
    }

    /// Lo que oye cada cliente mientras pasa `frames` tramas de 20 ms con `each`.
    /// Antes vacía lo que quedase de la medida anterior en el búfer de cada cliente.
    fn listen(&self, frames: usize, mut each: impl FnMut(usize)) -> Vec<f32> {
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        std::thread::sleep(Duration::from_millis(300));
        for c in &self.clients {
            for _ in 0..30 {
                c.render(&mut out);
            }
        }
        let mut energy = vec![0f32; self.clients.len()];
        let start = Instant::now();
        for n in 0..frames {
            each(n);
            let next = start + Duration::from_millis(20) * (n as u32 + 1);
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            for (i, c) in self.clients.iter().enumerate() {
                c.render(&mut out);
                energy[i] += out.iter().map(|s| s * s).sum::<f32>();
            }
        }
        energy
    }

    /// Habla el cliente `who` durante 40 tramas.
    fn speak(&self, who: usize) -> Vec<f32> {
        let speaker = &self.clients[who];
        speaker.set_push_to_talk(true);
        let e = self.listen(40, |n| speaker.push_capture(&sine_frame(n)));
        speaker.set_push_to_talk(false);
        e
    }

    /// Pasa 40 tramas por un canal.
    fn play(&self, channel: &[u8; 16]) -> Vec<f32> {
        self.listen(40, |n| assert!(self.server.channel_send_pcm(channel, &sine_frame(n))))
    }
}

const HEARD: f32 = 1.0;

#[test]
fn a_channel_at_a_point_is_heard_only_within_its_range() {
    let s = Scene::new(&[5.0, 200.0]);
    let ch = s.server.create_channel(Target::Point { pos: [0.0, 64.0, 0.0], dim: OVERWORLD }, 48.0, 48_000, true).unwrap();
    let e = s.play(&ch);
    assert!(e[0] > HEARD, "a 5 bloques se oye: {}", e[0]);
    assert!(e[1] < 1e-6, "a 200 bloques no: {}", e[1]);
    // No es un jugador: no sale en quién habla.
    assert!(s.clients[0].speaking_players().iter().all(|(who, _)| !channels::is_virtual(who)));
}

#[test]
fn a_direct_channel_reaches_only_its_listeners_at_any_distance() {
    let s = Scene::new(&[0.0, 1.0, 5000.0]);
    let ch = s.server.create_channel(Target::Listeners(HashSet::from([id(2)])), 1.0, 48_000, false).unwrap();
    let e = s.play(&ch);
    assert!(e[2] > HEARD, "el oyente lejano lo oye");
    assert!(e[0] < 1e-6 && e[1] < 1e-6, "los demás no: {e:?}");
}

#[test]
fn a_group_channel_follows_the_group_members() {
    let s = Scene::new(&[0.0, 1.0]);
    s.server.create_group(&id(1), "Radio", None).unwrap();
    let ch = s.server.create_channel(Target::Group("radio".into()), 1.0, 48_000, false).unwrap();
    let e = s.play(&ch);
    assert!(e[1] > HEARD && e[0] < 1e-6, "solo el del grupo: {e:?}");
    // Al salir del grupo deja de oírlo.
    assert!(s.server.leave_group(&id(1)));
    let e = s.play(&ch);
    assert!(e[1] < 1e-6, "ya no está en el grupo: {e:?}");
}

#[test]
fn a_channel_can_follow_a_player() {
    let s = Scene::new(&[0.0, 300.0, 305.0]);
    let ch = s.server.create_channel(Target::Follow(id(1)), 32.0, 48_000, false).unwrap();
    let e = s.play(&ch);
    assert!(e[1] > HEARD && e[2] > HEARD, "le oyen él y quien está a su lado: {e:?}");
    assert!(e[0] < 1e-6, "lejos no: {e:?}");
    assert!(s.server.remove_channel(&ch));
    assert!(!s.server.channel_send_pcm(&ch, &sine_frame(0)), "un canal borrado no envía");
}

#[test]
fn a_range_override_works_like_a_megaphone() {
    let s = Scene::new(&[0.0, 150.0]);
    assert!(s.speak(0)[1] < 1e-6, "a 150 bloques no llega la voz normal");
    s.server.set_range_override(&id(0), Some(200.0));
    assert!(s.speak(0)[1] > HEARD, "con alcance 200, sí");
    s.server.set_range_override(&id(0), None);
    assert!(s.speak(0)[1] < 1e-6, "al quitarlo vuelve al normal");
}

#[test]
fn hush_leaves_only_the_priority_voices() {
    let s = Scene::new(&[0.0, 2.0, 4.0]);
    s.server.set_priority(&id(0), true);
    s.server.set_hush(true);
    assert!(s.speak(1)[2] < 1e-6, "el público calla");
    assert!(s.speak(0)[2] > HEARD, "el anfitrión se oye");
    s.server.set_hush(false);
    assert!(s.speak(1)[2] > HEARD, "al acabar, todos otra vez");
}

#[test]
fn plugins_can_listen_to_the_microphone() {
    let s = Scene::new(&[0.0, 2.0]);
    s.speak(0);
    assert!(s.server.poll_mic().is_empty(), "sin escucha no se copia nada");

    s.clients[0].set_talk_mode(TalkMode::Shout);
    s.server.set_mic_tap(true);
    // Sin lo que quedara en camino de la frase anterior, que era en modo normal.
    drain_mic(&s.server);
    s.speak(0);
    let frames = s.server.poll_mic();
    assert!(frames.len() >= 30, "llegan las tramas: {}", frames.len());
    let odd: Vec<_> = frames.iter().filter(|f| f.player != id(0) || f.talk != TalkMode::Shout || f.group).map(|f| (f.seq, f.talk)).collect();
    assert!(odd.is_empty(), "todas del que habla, gritando: {odd:?}");
    // Y se pueden decodificar.
    let mut dec = VoiceDecoder::new().unwrap();
    let mut pcm = [0f32; FRAME_SAMPLES];
    let energy: f32 = frames
        .iter()
        .skip(5)
        .map(|f| {
            dec.decode(&f.opus, &mut pcm);
            pcm.iter().map(|x| x * x).sum::<f32>()
        })
        .sum();
    assert!(energy > HEARD, "es la voz: {energy}");

    // Un silenciado no sale. Primero el silencio, que corta la copia, y luego se tira lo
    // que quedara en camino de la frase anterior.
    s.server.set_muted(&id(0), true);
    drain_mic(&s.server);
    s.speak(0);
    assert!(s.server.poll_mic().is_empty(), "lo silenciado no se copia");
    s.server.set_mic_tap(false);
}

#[test]
fn connect_speak_and_disconnect_events_arrive_in_order() {
    let s = Scene::new(&[0.0, 2.0]);
    let kinds = |s: &Scene| -> Vec<(EventKind, [u8; 16], String)> {
        s.server.poll_events().into_iter().map(|e| (e.kind, e.player, e.detail)).collect()
    };
    let joined = kinds(&s);
    for n in 0..2 {
        assert!(joined.contains(&(EventKind::Connected, id(n), String::new())), "{joined:?}");
    }

    s.clients[0].set_talk_mode(TalkMode::Whisper);
    s.speak(0);
    std::thread::sleep(Duration::from_millis(700)); // ACTIVE_SPEAKER y el barrido
    let talked = kinds(&s);
    assert_eq!(
        talked,
        vec![
            (EventKind::StartedSpeaking, id(0), "whisper".into()),
            (EventKind::StoppedSpeaking, id(0), String::new()),
        ]
    );

    s.server.remove_player(&id(1));
    assert_eq!(kinds(&s), vec![(EventKind::Disconnected, id(1), String::new())]);
}

#[test]
fn groups_can_be_managed_without_a_password_or_an_owner() {
    let s = Scene::new(&[0.0, 2.0]);
    s.server.create_group(&id(0), "Escenario", Some("secreta")).unwrap();
    assert!(s.server.join_group(&id(1), "escenario", None).is_err(), "con contraseña no entra");
    s.server.force_join_group(&id(1), "escenario").unwrap();
    assert_eq!(s.server.group_of(&id(1)).as_deref(), Some("Escenario"));
    let (owner, password, persistent) = s.server.group_info("ESCENARIO").unwrap();
    assert_eq!((owner, password, persistent), (Some(id(0)), true, false));

    s.server.set_group_persistent_by_name("escenario", true).unwrap();
    assert!(s.server.group_info("escenario").unwrap().2);
    // Permanente y vacío se queda; temporal y vacío se borra.
    s.server.leave_group(&id(0));
    s.server.leave_group(&id(1));
    assert!(s.server.group_info("escenario").is_some());
    s.server.set_group_persistent_by_name("escenario", false).unwrap();
    assert!(s.server.group_info("escenario").is_none());
}
