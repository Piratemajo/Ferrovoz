//! Pruebas de extremo a extremo sin hardware: servidor real por UDP en localhost y
//! varios clientes. Cifrado, distancia, dimensión, paneo, etc.

use ferrovoz::client::{TransmitMode, VoiceClient};
use ferrovoz::protocol::{FRAME_SAMPLES, SAMPLE_RATE};
use ferrovoz::server::VoiceServer;
use std::time::{Duration, Instant};

const OVERWORLD: i32 = 1;
const NETHER: i32 = 2;

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(5), "timeout esperando: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sine_frame(n: usize) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 300.0 * (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

struct Energy {
    left: f32,
    right: f32,
}

impl Energy {
    fn total(&self) -> f32 {
        self.left + self.right
    }
}

#[test]
fn proximity_voice_end_to_end() {
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();

    let speaker_id = [1u8; 16];
    let near_id = [2u8; 16];
    let far_id = [3u8; 16];
    let nether_id = [4u8; 16];

    // El hablante está 10 bloques al este del oyente cercano.
    let ids = [speaker_id, near_id, far_id, nether_id];
    let keys: Vec<_> = ids.iter().map(|id| server.add_player(*id)).collect();
    server.update_player(&speaker_id, [110.0, 64.0, 0.0], OVERWORLD);
    server.update_player(&near_id, [100.0, 64.0, 0.0], OVERWORLD);
    server.update_player(&far_id, [400.0, 64.0, 0.0], OVERWORLD);
    server.update_player(&nether_id, [100.0, 64.0, 0.0], NETHER);

    let clients: Vec<VoiceClient> = ids
        .iter()
        .zip(&keys)
        .map(|(id, key)| VoiceClient::start(addr, *id, *key, false).unwrap())
        .collect();
    let (speaker, listeners) = clients.split_first().unwrap();

    wait_until("todos conectados", || clients.iter().all(|c| c.is_connected()));
    assert_eq!(server.connected_count(), 4);

    // Los tonos sintéticos no son voz: sin esto la puerta de ruido los cortaría.
    for c in &clients {
        c.set_noise_suppression(false);
    }
    speaker.set_mode(TransmitMode::PushToTalk);
    speaker.set_push_to_talk(true);
    listeners[0].set_listener_yaw(0.0); // mirando al sur, el este queda a la izquierda

    let mut energy: Vec<Energy> = listeners.iter().map(|_| Energy { left: 0.0, right: 0.0 }).collect();
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    let mut heard_speaking = false;

    for n in 0..60 {
        speaker.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(20));
        for (l, e) in listeners.iter().zip(energy.iter_mut()) {
            l.render(&mut out);
            for lr in out.chunks_exact(2) {
                e.left += lr[0] * lr[0];
                e.right += lr[1] * lr[1];
            }
        }
        heard_speaking |= listeners[0].speaking_players().iter().any(|(id, _)| *id == speaker_id);
    }

    assert!(speaker.is_talking());
    let (near, far, nether) = (&energy[0], &energy[1], &energy[2]);
    println!(
        "energía -> cercano L={:.2} R={:.2} | lejano={:.4} | nether={:.4}",
        near.left, near.right, far.total(), nether.total()
    );
    assert!(near.total() > 50.0, "el oyente cercano debería oír la voz");
    assert!(near.left > near.right * 4.0, "el hablante al este debe sonar por la izquierda");
    assert_eq!(far.total(), 0.0, "a 290 bloques no debería llegar nada");
    assert_eq!(nether.total(), 0.0, "otra dimensión no debería oír nada");
    assert!(heard_speaking, "el oyente debería ver al hablante en la lista de quien habla");

    // Soltar la tecla corta la transmisión.
    speaker.set_push_to_talk(false);
    speaker.push_capture(&sine_frame(0));
    assert!(!speaker.is_talking());

    let stats = server.stats();
    println!(
        "servidor: {} paquetes in, {} out, {} rechazados",
        stats.packets_in.load(std::sync::atomic::Ordering::Relaxed),
        stats.packets_out.load(std::sync::atomic::Ordering::Relaxed),
        stats.rejected.load(std::sync::atomic::Ordering::Relaxed),
    );
}

#[test]
fn impostor_with_wrong_key_is_ignored() {
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let victim = [7u8; 16];
    let _real_key = server.add_player(victim);

    // Conoce el UUID pero no la clave.
    let impostor = VoiceClient::start(addr, victim, [0u8; 32], false).unwrap();
    std::thread::sleep(Duration::from_millis(700));
    assert!(!impostor.is_connected());
    assert_eq!(server.connected_count(), 0);
    assert!(server.stats().rejected.load(std::sync::atomic::Ordering::Relaxed) > 0);
}

#[test]
fn voice_activation_mode() {
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let id = [9u8; 16];
    let key = server.add_player(id);
    let c = VoiceClient::start(addr, id, key, false).unwrap();
    wait_until("conectado", || c.is_connected());

    c.set_noise_suppression(false);
    c.set_mode(TransmitMode::VoiceActivation);
    c.push_capture(&vec![0.0; FRAME_SAMPLES]);
    assert!(!c.is_talking(), "silencio no debe activar la voz");
    c.push_capture(&sine_frame(0));
    assert!(c.is_talking(), "voz fuerte debe activar la transmisión");
    // El hangover mantiene la transmisión unos instantes tras callar.
    c.push_capture(&vec![0.0; FRAME_SAMPLES]);
    assert!(c.is_talking());
    for _ in 0..20 {
        c.push_capture(&vec![0.0; FRAME_SAMPLES]);
    }
    assert!(!c.is_talking());

    c.set_muted(true);
    c.push_capture(&sine_frame(1));
    assert!(!c.is_talking(), "silenciado no transmite");
}

// ---------------------------------------------------------------------------
// Alcances, grupos, moderación, permisos y túnel
// ---------------------------------------------------------------------------

use ferrovoz::protocol::TalkMode;

/// Servidor real y varios clientes, con utilidades para hablar y medir.
struct Scene {
    server: VoiceServer,
    clients: Vec<VoiceClient>,
    ids: Vec<[u8; 16]>,
}

impl Scene {
    /// `positions[i]` = (x, dimensión) de cada jugador.
    fn new(positions: &[(f64, i32)]) -> Self {
        let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
        let addr = server.local_addr().unwrap();
        let ids: Vec<[u8; 16]> = (0..positions.len()).map(|i| [i as u8 + 10; 16]).collect();
        let clients = ids
            .iter()
            .zip(positions)
            .map(|(id, (x, dim))| {
                let key = server.add_player(*id);
                server.update_player(id, [*x, 64.0, 0.0], *dim);
                let c = VoiceClient::start(addr, *id, key, false).unwrap();
                c.set_noise_suppression(false);
                c
            })
            .collect::<Vec<_>>();
        wait_until("conectados", || clients.iter().all(|c| c.is_connected()));
        Self { server, clients, ids }
    }

    /// Habla `who` y devuelve la energía que recibe cada uno de los demás.
    fn speak(&self, who: usize, talk: TalkMode) -> Vec<f32> {
        let speaker = &self.clients[who];
        speaker.set_talk_mode(talk);
        speaker.set_push_to_talk(true);
        let mut e = vec![0.0f32; self.clients.len()];
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        for n in 0..40 {
            speaker.push_capture(&sine_frame(n));
            std::thread::sleep(Duration::from_millis(20));
            for (i, c) in self.clients.iter().enumerate() {
                if i == who {
                    continue;
                }
                c.render(&mut out);
                e[i] += out.iter().map(|s| s * s).sum::<f32>();
            }
        }
        speaker.set_push_to_talk(false);
        // Vaciar para la siguiente prueba.
        std::thread::sleep(Duration::from_millis(100));
        for c in &self.clients {
            for _ in 0..30 {
                c.render(&mut out);
            }
        }
        e
    }
}

#[test]
fn whisper_normal_and_shout_ranges() {
    // Oyentes a 5, 30 y 70 bloques del jugador 0.
    let s = Scene::new(&[(0.0, 1), (5.0, 1), (30.0, 1), (70.0, 1)]);
    let heard = |e: &[f32]| e[1..].iter().map(|v| *v > 1.0).collect::<Vec<_>>();
    assert_eq!(heard(&s.speak(0, TalkMode::Whisper)), [true, false, false], "susurro: solo a 5 bloques");
    assert_eq!(heard(&s.speak(0, TalkMode::Normal)), [true, true, false], "normal: hasta 48");
    assert_eq!(heard(&s.speak(0, TalkMode::Shout)), [true, true, true], "grito: hasta 96");

    // Sin permiso para gritar, cuenta como voz normal.
    s.server.set_permissions(&s.ids[0], true, false);
    assert_eq!(heard(&s.speak(0, TalkMode::Shout)), [true, true, false], "grito sin permiso = normal");
}

#[test]
fn groups_reach_any_distance_and_dimension_but_only_members() {
    // 1 en el Nether a 10000 bloques (miembro), 2 al lado pero fuera del grupo.
    let s = Scene::new(&[(0.0, 1), (10_000.0, 2), (3.0, 1)]);
    s.server.create_group(&s.ids[0], "amigos", Some("clave")).unwrap();
    s.server.join_group(&s.ids[1], "amigos", Some("clave")).unwrap();
    let e = s.speak(0, TalkMode::Normal);
    assert!(e[1] > 1.0, "el miembro lejano y en otra dimensión oye");
    assert!(e[2] < 1e-6, "el vecino que no es del grupo no oye la voz de grupo");

    // Al salir del grupo vuelve la proximidad.
    s.server.leave_group(&s.ids[0]);
    let e = s.speak(0, TalkMode::Normal);
    assert!(e[1] < 1e-6 && e[2] > 1.0);
}

#[test]
fn admin_mute_and_speak_permission_block_voice() {
    let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
    s.server.set_muted(&s.ids[0], true);
    assert!(s.speak(0, TalkMode::Normal)[1] < 1e-6, "silenciado por un admin");
    s.server.set_muted(&s.ids[0], false);
    assert!(s.speak(0, TalkMode::Normal)[1] > 1.0);
    s.server.set_permissions(&s.ids[0], false, false);
    assert!(s.speak(0, TalkMode::Normal)[1] < 1e-6, "sin permiso para hablar");
}

#[test]
fn voice_works_through_the_tunnel() {
    use ferrovoz::client::TransportPolicy;
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let (a, b) = ([1u8; 16], [2u8; 16]);
    let (ka, kb) = (server.add_player(a), server.add_player(b));
    server.update_player(&a, [0.0, 64.0, 0.0], 1);
    server.update_player(&b, [4.0, 64.0, 0.0], 1);
    // `a` por túnel (UDP bloqueado), `b` por UDP.
    let ca = VoiceClient::start_with(addr, a, ka, false, TransportPolicy::TunnelOnly).unwrap();
    let cb = VoiceClient::start(addr, b, kb, false).unwrap();
    for c in [&ca, &cb] {
        c.set_noise_suppression(false);
    }

    // Lo que hace Java cada tick: mover paquetes entre cliente y servidor.
    let pump = || {
        for p in ca.poll_tunnel() {
            server.handle_tunnel(&a, &p);
        }
        for p in server.poll_tunnel(&a) {
            ca.handle_tunnel(&p);
        }
    };
    wait_until("ambos conectados", || {
        pump();
        ca.is_connected() && cb.is_connected()
    });
    assert!(ca.net_stats().tunnel && !cb.net_stats().tunnel);
    assert_eq!(server.tunnel_count(), 1);

    // a (túnel) -> b (UDP) y b (UDP) -> a (túnel)
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    let (mut heard_b, mut heard_a) = (0.0f32, 0.0f32);
    ca.set_push_to_talk(true);
    cb.set_push_to_talk(true);
    for n in 0..40 {
        ca.push_capture(&sine_frame(n));
        cb.push_capture(&sine_frame(n));
        pump();
        std::thread::sleep(Duration::from_millis(20));
        pump();
        cb.render(&mut out);
        heard_b += out.iter().map(|s| s * s).sum::<f32>();
        ca.render(&mut out);
        heard_a += out.iter().map(|s| s * s).sum::<f32>();
    }
    println!("túnel: b oye {heard_b:.1}, a oye {heard_a:.1}");
    assert!(heard_b > 10.0, "la voz que sube por el túnel llega a un jugador UDP");
    assert!(heard_a > 10.0, "la voz baja por el túnel");
}

#[test]
fn auto_transport_falls_back_to_tunnel_when_udp_is_blocked() {
    // Servidor al que el UDP no llega: puerto sin nadie escuchando.
    let dead = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    let real = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let id = [7u8; 16];
    let key = real.add_player(id);
    let c = VoiceClient::start(dead_addr, id, key, false).unwrap();
    let start = Instant::now();
    wait_until("conectado por túnel", || {
        for p in c.poll_tunnel() {
            real.handle_tunnel(&id, &p);
        }
        for p in real.poll_tunnel(&id) {
            c.handle_tunnel(&p);
        }
        c.is_connected()
    });
    assert!(c.net_stats().tunnel);
    assert!(start.elapsed() >= Duration::from_secs(3), "primero intenta UDP unos segundos");
}

#[test]
fn adaptive_bitrate_drops_on_packet_loss() {
    use std::net::UdpSocket;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let server_addr = server.local_addr().unwrap();

    // Proxy UDP que tira 1 de cada 3 paquetes de voz de subida.
    let proxy = UdpSocket::bind("127.0.0.1:0").unwrap();
    proxy.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let upstream = UdpSocket::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    upstream.connect(server_addr).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let relay = std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        let mut client = None;
        let mut n = 0u32;
        while !stop2.load(Ordering::Relaxed) {
            if let Ok((len, from)) = proxy.recv_from(&mut buf) {
                client = Some(from);
                let is_audio = len > 3 && buf[3] == 2; // tipo AudioUp
                n += 1;
                if !(is_audio && n.is_multiple_of(3)) {
                    let _ = upstream.send(&buf[..len]);
                }
            }
            while let Ok(len) = upstream.recv(&mut buf) {
                if let Some(c) = client {
                    let _ = proxy.send_to(&buf[..len], c);
                }
            }
        }
    });

    let id = [9u8; 16];
    let key = server.add_player(id);
    let c = VoiceClient::start(proxy_addr, id, key, false).unwrap();
    c.set_noise_suppression(false);
    wait_until("conectado a través del proxy", || c.is_connected());
    assert_eq!(c.net_stats().bitrate, 32_000, "red perfecta al principio: máxima calidad");

    c.set_push_to_talk(true);
    for n in 0..400 {
        c.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(2));
    }
    wait_until("el servidor mide la pérdida", || server.loss_percent(&id).unwrap() > 0);
    let loss = server.loss_percent(&id).unwrap();
    println!("pérdida medida por el servidor: {loss} %");
    assert!((25..=40).contains(&loss), "≈33 %: {loss}");

    // El siguiente ping trae la pérdida y el cliente baja el bitrate.
    wait_until("el cliente se adapta", || c.net_stats().bitrate < 32_000);
    let st = c.net_stats();
    println!("cliente: pérdida {} %, bitrate {} bps, rtt {} ms", st.loss_percent, st.bitrate, st.rtt_ms);
    assert_eq!(st.bitrate, 14_000);

    stop.store(true, Ordering::Relaxed);
    relay.join().unwrap();
}

// ---------------------------------------------------------------------------
// Seguridad (replay, flood), estado de grupo, reverb del hablante, oclusión del servidor
// ---------------------------------------------------------------------------

/// Proxy UDP transparente que guarda copia de todo lo que sube.
struct SniffingProxy {
    addr: std::net::SocketAddr,
    captured: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SniffingProxy {
    fn new(server: std::net::SocketAddr) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        let proxy = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        proxy.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
        let addr = proxy.local_addr().unwrap();
        let upstream = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        upstream.set_nonblocking(true).unwrap();
        upstream.connect(server).unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (cap, st) = (captured.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            let mut client = None;
            while !st.load(Ordering::Relaxed) {
                if let Ok((len, from)) = proxy.recv_from(&mut buf) {
                    client = Some(from);
                    cap.lock().unwrap().push(buf[..len].to_vec());
                    let _ = upstream.send(&buf[..len]);
                }
                while let Ok(len) = upstream.recv(&mut buf) {
                    if let Some(c) = client {
                        let _ = proxy.send_to(&buf[..len], c);
                    }
                }
            }
        });
        Self { addr, captured, stop, thread: Some(thread) }
    }
}

impl Drop for SniffingProxy {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Socket en 127.0.0.2. Linux y Windows la tienen; macOS necesita
/// `ifconfig lo0 alias 127.0.0.2` (lo hace el CI).
fn second_loopback() -> Option<std::net::UdpSocket> {
    std::net::UdpSocket::bind("127.0.0.2:0").ok()
}

#[test]
fn replayed_packets_are_rejected_and_cannot_hijack_voice() {
    use std::sync::atomic::Ordering;
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let proxy = SniffingProxy::new(addr);
    let (victim, listener) = ([1u8; 16], [2u8; 16]);
    let (kv, kl) = (server.add_player(victim), server.add_player(listener));
    server.update_player(&victim, [0.0, 64.0, 0.0], 1);
    server.update_player(&listener, [3.0, 64.0, 0.0], 1);
    let cv = VoiceClient::start(proxy.addr, victim, kv, false).unwrap();
    let cl = VoiceClient::start(addr, listener, kl, false).unwrap();
    for c in [&cv, &cl] {
        c.set_noise_suppression(false);
    }
    wait_until("conectados", || cv.is_connected() && cl.is_connected());

    // La víctima habla y el atacante captura todo lo que sube.
    cv.set_push_to_talk(true);
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    for n in 0..20 {
        cv.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(20));
        cl.render(&mut out);
    }
    cv.set_push_to_talk(false);
    std::thread::sleep(Duration::from_millis(100));
    let captured = proxy.captured.lock().unwrap().clone();
    assert!(captured.len() >= 20);
    let before = server.stats().replayed.load(Ordering::Relaxed);

    // 1) Reenviar desde otra dirección: todo rechazado.
    //    (127.0.0.2 si existe; si no, otro puerto de 127.0.0.1.)
    let attacker = second_loopback().unwrap_or_else(|| std::net::UdpSocket::bind("127.0.0.1:0").unwrap());
    for pkt in &captured {
        attacker.send_to(pkt, addr).unwrap();
    }
    std::thread::sleep(Duration::from_millis(200));
    let replayed = server.stats().replayed.load(Ordering::Relaxed) - before;
    println!("paquetes repetidos rechazados: {replayed} de {}", captured.len());
    assert!(replayed as usize >= captured.len() - 2, "casi todos (los no limitados por IP) se detectan como replay");

    // 2) El Hello repetido no ha desviado la voz: la víctima sigue recibiendo y el
    //    atacante no recibe nada.
    attacker.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    cl.set_push_to_talk(true);
    let mut heard_by_victim = 0.0;
    for n in 0..30 {
        cl.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(20));
        cv.render(&mut out);
        heard_by_victim += out.iter().map(|s| s * s).sum::<f32>();
    }
    let mut buf = [0u8; 1500];
    assert!(attacker.recv(&mut buf).is_err(), "el atacante no recibe la voz de nadie");
    assert!(heard_by_victim > 10.0, "la víctima sigue oyendo con normalidad");
}

#[test]
fn flooding_is_rate_limited_and_others_keep_working() {
    use ferrovoz::protocol::{self, AudioUp, Environment, PacketType};
    use std::sync::atomic::Ordering;
    let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
    // Cliente modificado con su clave legítima que manda 3000 tramas de golpe.
    let flooder_id = [0xAAu8; 16];
    let key = s.server.add_player(flooder_id);
    s.server.update_player(&flooder_id, [1.0, 64.0, 0.0], 1);
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = s.server.local_addr().unwrap();
    sock.send_to(&protocol::seal(&key, PacketType::Hello, Some(&flooder_id), &[], 1), addr).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    for i in 0..3000u64 {
        let body = AudioUp { seq: i as u32, talk: TalkMode::Normal, env: Environment::default(), opus: vec![0; 40] }.encode();
        let _ = sock.send_to(&protocol::seal(&key, PacketType::AudioUp, Some(&flooder_id), &body, i + 2), addr);
        // En tandas: todo de golpe con la máquina cargada, el SO tira parte antes de
        // llegar al servidor y no contaría como limitado.
        if i % 100 == 99 {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    let limited = s.server.stats().rate_limited.load(Ordering::Relaxed);
    println!("tramas descartadas por el límite: {limited} de 3000");
    assert!(limited >= 2000, "sin límite pasarían las 3 000: {limited}");
    // Los demás siguen hablando sin problema.
    assert!(s.speak(0, TalkMode::Normal)[1] > 1.0);
}

#[test]
fn garbage_from_an_ip_gets_that_ip_ignored() {
    use std::sync::atomic::Ordering;
    let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
    let addr = s.server.local_addr().unwrap();
    // Necesita una IP distinta de la de los jugadores.
    let Some(junk) = second_loopback() else {
        println!("SALTADO: no hay 127.0.0.2 (macOS: sudo ifconfig lo0 alias 127.0.0.2 up)");
        return;
    };
    for i in 0..500u32 {
        let mut p = vec![b'F', b'V', 3, 2];
        p.extend_from_slice(&[0x55; 16]); // UUID inventado
        p.extend_from_slice(&i.to_le_bytes());
        p.extend_from_slice(&[0; 40]);
        junk.send_to(&p, addr).unwrap();
        // En tandas de 50, por lo mismo que arriba.
        if i % 50 == 49 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    std::thread::sleep(Duration::from_millis(200));
    let limited = s.server.stats().rate_limited.load(Ordering::Relaxed);
    println!("paquetes basura ignorados sin procesar: {limited} de 500");
    assert!(limited > 300, "{limited}");
    assert!(s.speak(0, TalkMode::Normal)[1] > 1.0, "los jugadores legítimos (otra IP) no se ven afectados");
}

#[test]
fn client_learns_its_group_for_the_hud() {
    let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
    s.server.create_group(&s.ids[0], "Piratas", None).unwrap();
    wait_until("estado de grupo recibido", || s.clients[0].group_state().name == "Piratas");
    assert!(s.clients[0].group_state().owner);
    s.server.join_group(&s.ids[1], "piratas", None).unwrap();
    wait_until("miembro informado", || s.clients[1].group_state().name == "Piratas");
    assert!(!s.clients[1].group_state().owner);
    s.server.leave_group(&s.ids[0]);
    wait_until("la corona pasa al otro miembro", || s.clients[1].group_state().owner);
    wait_until("el que sale ya no tiene grupo", || s.clients[0].group_state().name.is_empty());
}

#[test]
fn speaker_in_a_cave_echoes_even_if_listener_is_outside() {
    let tail_heard = |speaker_wet: f32| {
        let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
        s.clients[0].set_environment(speaker_wet, 1.0); // hablante en una cueva grande (o no)
        s.clients[1].set_environment(0.0, 0.0); // oyente al aire libre
        let speaker = &s.clients[0];
        speaker.set_push_to_talk(true);
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        for n in 0..30 {
            speaker.push_capture(&sine_frame(n));
            std::thread::sleep(Duration::from_millis(20));
            s.clients[1].render(&mut out);
        }
        speaker.set_push_to_talk(false);
        // ¿Queda cola al callar?
        std::thread::sleep(Duration::from_millis(150));
        let mut tail = 0.0;
        for i in 0..40 {
            s.clients[1].render(&mut out);
            if i >= 15 {
                tail += out.iter().map(|x| x * x).sum::<f32>();
            }
        }
        tail
    };
    let outside = tail_heard(0.0);
    let cave = tail_heard(0.45);
    println!("cola al oyente: quien habla fuera {outside:.5}, quien habla en cueva {cave:.5}");
    assert!(outside < 1e-6);
    assert!(cave > 0.005, "la voz de quien está en una cueva llega con eco");
}

#[test]
fn server_side_walls_muffle_far_speakers() {
    // Tono de 3 kHz: la oclusión se nota sobre todo en agudos.
    let tone = |n: usize| -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 3000.0 * (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    };
    let heard = |wall: bool| {
        let s = Scene::new(&[(0.0, 1), (20.0, 1)]);
        let speaker = &s.clients[0];
        speaker.set_push_to_talk(true);
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        let mut e = 0.0;
        for n in 0..40 {
            if n >= 3 {
                // Lo que haría Java: calcular los bloques entre ellos y renovarlo
                // (cada 4 ticks en el juego; caducan a 1 s).
                let pairs = s.server.active_pairs();
                assert!(pairs.contains(&(s.ids[0], s.ids[1])), "el par está activo");
                if wall {
                    s.server.set_occlusion(&s.ids[0], &s.ids[1], 1.0);
                }
            }
            speaker.push_capture(&tone(n));
            std::thread::sleep(Duration::from_millis(20));
            s.clients[1].render(&mut out);
            if n >= 15 {
                e += out.iter().map(|x| x * x).sum::<f32>();
            }
        }
        e
    };
    let open = heard(false);
    let walled = heard(true);
    println!("agudos a 20 bloques: sin pared {open:.2}, con pared (servidor) {walled:.2}");
    assert!(walled < open * 0.1, "la pared calculada por el servidor apaga la voz");
}

/// Energía de agudos (3 kHz) que oye el jugador 1 cuando habla el 0.
fn treble_heard(setup: impl Fn(&Scene)) -> f32 {
    let tone = |n: usize| -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 3000.0 * (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    };
    let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
    setup(&s);
    let speaker = &s.clients[0];
    speaker.set_push_to_talk(true);
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    let mut e = 0.0;
    for n in 0..40 {
        // El servidor actualiza la posición (y si está bajo el agua) cada tick.
        speaker.push_capture(&tone(n));
        std::thread::sleep(Duration::from_millis(20));
        s.clients[1].render(&mut out);
        if n >= 15 {
            e += out.iter().map(|x| x * x).sum::<f32>();
        }
    }
    e
}

#[test]
fn speaker_underwater_sounds_like_water_unless_the_listener_turns_it_off() {
    let dry = treble_heard(|_| {});
    let under = treble_heard(|s| s.server.update_player_full(&s.ids[0], [0.0, 64.0, 0.0], OVERWORLD, true));
    let under_but_off = treble_heard(|s| {
        s.server.update_player_full(&s.ids[0], [0.0, 64.0, 0.0], OVERWORLD, true);
        s.clients[1].set_underwater_effect(false);
    });
    let listener_under = treble_heard(|s| s.clients[1].set_underwater(true));
    println!("agudos: normal {dry:.2}, quien habla bajo el agua {under:.4}, efecto desactivado {under_but_off:.2}, tú bajo el agua {listener_under:.4}");
    assert!(under < dry * 0.01, "la voz de quien está bajo el agua llega tapada");
    assert!(under_but_off > dry * 0.5, "con el efecto desactivado se oye normal");
    assert!(listener_under < dry * 0.01, "si tú estás bajo el agua, también oyes tapado");
}

#[test]
fn the_server_can_turn_the_underwater_effect_off_for_everyone() {
    let dry = treble_heard(|_| {});
    // Los dos bajo el agua y con el efecto activado en sus ajustes, pero el servidor lo quita.
    let off = treble_heard(|s| {
        s.server.set_underwater_effect(false);
        s.server.update_player_full(&s.ids[0], [0.0, 64.0, 0.0], OVERWORLD, true);
        s.clients[1].set_underwater(true);
        wait_until("el oyente sabe que no hay efecto", || !s.clients[1].underwater_allowed());
    });
    println!("agudos: normal {dry:.2}, bajo el agua sin efecto del servidor {off:.2}");
    assert!(off > dry * 0.5, "se oye normal aunque los dos estén bajo el agua");
}

#[test]
fn cave_echo_can_be_turned_off() {
    let tail = |echo: bool| {
        let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
        s.clients[0].set_environment(0.45, 1.0); // hablante en una cueva grande
        s.clients[1].set_cave_echo(echo);
        let speaker = &s.clients[0];
        speaker.set_push_to_talk(true);
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        for n in 0..30 {
            speaker.push_capture(&sine_frame(n));
            std::thread::sleep(Duration::from_millis(20));
            s.clients[1].render(&mut out);
        }
        speaker.set_push_to_talk(false);
        std::thread::sleep(Duration::from_millis(150));
        let mut tail = 0.0;
        for i in 0..40 {
            s.clients[1].render(&mut out);
            if i >= 15 {
                tail += out.iter().map(|x| x * x).sum::<f32>();
            }
        }
        tail
    };
    let (on, off) = (tail(true), tail(false));
    println!("cola del eco: activado {on:.5}, desactivado {off:.7}");
    assert!(on > 0.005);
    assert!(off < 1e-6, "sin eco no queda cola");
}

#[test]
fn the_server_can_turn_cave_echo_off_for_everyone() {
    let s = Scene::new(&[(0.0, 1), (3.0, 1)]);
    s.server.set_cave_echo(false);
    // Los dos en una cueva grande y con el eco activado en sus ajustes.
    s.clients[0].set_environment(0.45, 1.0);
    s.clients[1].set_environment(0.45, 1.0);
    wait_until("el oyente sabe que no hay eco", || !s.clients[1].cave_echo_allowed());
    let speaker = &s.clients[0];
    speaker.set_push_to_talk(true);
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    let mut heard = 0.0;
    for n in 0..30 {
        speaker.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(20));
        s.clients[1].render(&mut out);
        heard += out.iter().map(|x| x * x).sum::<f32>();
    }
    speaker.set_push_to_talk(false);
    std::thread::sleep(Duration::from_millis(150));
    let mut tail = 0.0;
    for i in 0..40 {
        s.clients[1].render(&mut out);
        if i >= 15 {
            tail += out.iter().map(|x| x * x).sum::<f32>();
        }
    }
    assert!(heard > 1.0, "la voz llega");
    assert!(tail < 1e-6, "sin cola de eco: {tail}");
    // Y al volver a permitirlo, los clientes se enteran.
    s.server.set_cave_echo(true);
    wait_until("el eco vuelve", || s.clients[1].cave_echo_allowed());
}

/// Desactivar la voz: el servidor deja de enviarle audio y deja de contar como
/// conectado. Al reactivarla (misma clave, cliente nuevo) todo vuelve.
#[test]
fn turning_voice_off_and_on_again() {
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let (a, b) = ([21u8; 16], [22u8; 16]);
    let (ka, kb) = (server.add_player(a), server.add_player(b));
    server.update_player(&a, [0.0, 64.0, 0.0], OVERWORLD);
    server.update_player(&b, [5.0, 64.0, 0.0], OVERWORLD);
    let speaker = VoiceClient::start(addr, a, ka, false).unwrap();
    speaker.set_noise_suppression(false);
    speaker.set_push_to_talk(true);
    let mut listener = VoiceClient::start(addr, b, kb, false).unwrap();
    wait_until("conectados", || speaker.is_connected() && listener.is_connected());
    assert_eq!(server.connected_count(), 2);

    // Bye: el servidor ya no lo cuenta ni le envía nada.
    listener.goodbye();
    listener.stop();
    wait_until("el servidor lo suelta", || server.connected_count() == 1);
    let out_before = server.stats().packets_out.load(std::sync::atomic::Ordering::Relaxed);
    for n in 0..20 {
        speaker.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));
    let out_after = server.stats().packets_out.load(std::sync::atomic::Ordering::Relaxed);
    assert!(out_after - out_before < 5, "le sigue mandando voz: {}", out_after - out_before);

    // Cliente nuevo con la misma clave: no se toma por replay.
    listener = VoiceClient::start(addr, b, kb, false).unwrap();
    wait_until("vuelve a conectar", || listener.is_connected());
    assert_eq!(server.connected_count(), 2);
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    let mut heard = 0f32;
    for n in 0..40 {
        speaker.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(20));
        listener.render(&mut out);
        heard += out.iter().map(|s| s * s).sum::<f32>();
    }
    assert!(heard > 10.0, "tras reactivar no se oye ({heard})");
}

/// Reglas de espectadores y muertos, por red real.
#[test]
fn spectators_and_dead_follow_the_server_rules() {
    use ferrovoz::roles::{Role, RoleRule, RoleRules};
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let ids = [[31u8; 16], [32u8; 16], [33u8; 16]]; // vivo, espectador, muerto
    let roles = [Role::Alive, Role::Spectator, Role::Dead];
    let clients: Vec<VoiceClient> = ids
        .iter()
        .zip(roles)
        .enumerate()
        .map(|(i, (id, role))| {
            let key = server.add_player(*id);
            server.update_player_with_role(id, [i as f64 * 3.0, 64.0, 0.0], OVERWORLD, false, role);
            let c = VoiceClient::start(addr, *id, key, false).unwrap();
            c.set_noise_suppression(false);
            c
        })
        .collect();
    wait_until("conectados", || clients.iter().all(|c| c.is_connected()));

    // Habla `who`; devuelve quién lo oye.
    let heard_by = |who: usize| -> Vec<bool> {
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        for c in &clients {
            for _ in 0..10 {
                c.render(&mut out); // vaciar
            }
        }
        clients[who].set_push_to_talk(true);
        let mut energy = [0f32; 3];
        for n in 0..30 {
            clients[who].push_capture(&sine_frame(n));
            std::thread::sleep(Duration::from_millis(20));
            for (i, c) in clients.iter().enumerate() {
                c.render(&mut out);
                energy[i] += out.iter().map(|s| s * s).sum::<f32>();
            }
        }
        clients[who].set_push_to_talk(false);
        std::thread::sleep(Duration::from_millis(150));
        energy.iter().enumerate().map(|(i, e)| i != who && *e > 1.0).collect()
    };

    // Espectadores solo escuchan; los muertos, solo entre ellos.
    server.set_role_rules(RoleRules { spectator: RoleRule::Listen, dead: RoleRule::Own });
    assert_eq!(heard_by(0), [false, true, false], "al vivo le oye el espectador, no el muerto");
    assert_eq!(heard_by(1), [false, false, false], "al espectador no le oye nadie");
    assert_eq!(heard_by(2), [false, false, false], "el muerto solo habla con fantasmas (no hay otro)");

    // Fantasmas: espectadores y muertos hablan entre ellos y oyen a los vivos.
    server.set_role_rules(RoleRules { spectator: RoleRule::Ghost, dead: RoleRule::Ghost });
    assert_eq!(heard_by(0), [false, true, true]);
    assert_eq!(heard_by(2), [false, true, false], "al muerto le oye el espectador, no el vivo");
}

/// Repeticiones: se graban las tramas (recibidas y propias) y al reproducir suenan
/// desde la posición de cada uno respecto a la cámara.
#[test]
fn voice_is_recorded_and_played_back_in_replays() {
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let (a, b) = ([41u8; 16], [42u8; 16]);
    let (ka, kb) = (server.add_player(a), server.add_player(b));
    server.update_player(&a, [0.0, 64.0, 0.0], OVERWORLD);
    server.update_player(&b, [4.0, 64.0, 0.0], OVERWORLD);
    let speaker = VoiceClient::start(addr, a, ka, false).unwrap();
    let recorder = VoiceClient::start(addr, b, kb, false).unwrap();
    speaker.set_noise_suppression(false);
    wait_until("conectados", || speaker.is_connected() && recorder.is_connected());

    // Sin grabar no se guarda nada.
    speaker.set_push_to_talk(true);
    for n in 0..5 {
        speaker.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(recorder.poll_recorded().is_empty());

    // Grabando: las de Ana (recibidas) y las propias.
    recorder.set_recording(true);
    speaker.set_recording(true);
    for n in 0..40 {
        speaker.push_capture(&sine_frame(n));
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(100));
    let received = recorder.poll_recorded();
    let own = speaker.poll_recorded();
    println!("grabadas: {} recibidas, {} propias", received.len(), own.len());
    assert!(received.len() >= 30, "recibidas {}", received.len());
    assert!(own.len() >= 35, "propias {}", own.len());
    assert_eq!(&own[0][..16], &a, "la propia va con tu UUID");

    // Reproduciendo: cámara mirando al sur, Ana 3 bloques al oeste (a la derecha).
    let replay = VoiceClient::start_local(false).unwrap();
    replay.set_hrtf(false);
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    let (mut l, mut r) = (0f32, 0f32);
    for frame in &received {
        replay.set_source_position(a, [-3.0, 0.0, 0.0]);
        assert!(replay.replay_feed(frame));
        replay.render(&mut out);
        l += out.iter().step_by(2).map(|s| s * s).sum::<f32>();
        r += out.iter().skip(1).step_by(2).map(|s| s * s).sum::<f32>();
    }
    println!("repetición: L {l:.1} R {r:.1}");
    assert!(r > 50.0 && r > l * 4.0, "suena, y por la derecha");
    assert!(!replay.replay_feed(&[1, 2, 3]), "basura: se ignora");
}

/// Indicadores: el servidor reparte quién tiene voz, quién está silenciado o
/// ensordecido. Quien no aparece no tiene el mod.
#[test]
fn everyone_sees_who_is_muted_deafened_or_without_voice() {
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let (a, b, c, d) = ([51u8; 16], [52u8; 16], [53u8; 16], [54u8; 16]);
    let ka = server.add_player(a);
    let kb = server.add_player(b);
    let kc = server.add_player(c);
    server.add_player(d); // tiene el mod pero ha desactivado la voz
    let ca = VoiceClient::start(addr, a, ka, false).unwrap();
    let cb = VoiceClient::start(addr, b, kb, false).unwrap();
    let cc = VoiceClient::start(addr, c, kc, false).unwrap();
    wait_until("conectados", || ca.is_connected() && cb.is_connected() && cc.is_connected());
    cb.set_muted(true);
    cc.set_deafened(true);

    let flags_of = |id: [u8; 16]| ca.peer_states().and_then(|s| s.into_iter().find(|(p, _)| *p == id).map(|(_, f)| f));
    wait_until("estados repartidos", || flags_of(b) == Some(1 | 4) && flags_of(c) == Some(2 | 4));
    assert_eq!(flags_of(a), Some(4), "tú: conectado, sin más");
    assert_eq!(flags_of(d), Some(0), "con el mod pero sin voz conectada");
    assert_eq!(flags_of([99; 16]), None, "quien no está no tiene Ferrovoz");

    // Un silencio de admin también se ve.
    server.set_muted(&a, true);
    wait_until("silenciado por admin", || cb.peer_states().is_some_and(|s| s.contains(&(a, 1 | 4))));
    // Al quitar el mute propio deja de verse.
    cb.set_muted(false);
    wait_until("ya no silenciado", || flags_of(b) == Some(4));
}

/// Volumen de grupo independiente del de proximidad.
#[test]
fn group_voice_has_its_own_volume() {
    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let addr = server.local_addr().unwrap();
    let (a, b) = ([61u8; 16], [62u8; 16]);
    let (ka, kb) = (server.add_player(a), server.add_player(b));
    server.update_player(&a, [0.0, 64.0, 0.0], OVERWORLD);
    server.update_player(&b, [3.0, 64.0, 0.0], OVERWORLD);
    let speaker = VoiceClient::start(addr, a, ka, false).unwrap();
    let listener = VoiceClient::start(addr, b, kb, false).unwrap();
    speaker.set_noise_suppression(false);
    wait_until("conectados", || speaker.is_connected() && listener.is_connected());
    let heard = || {
        speaker.set_push_to_talk(true);
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        for _ in 0..50 {
            listener.render(&mut out);
        }
        let mut e = 0f32;
        for n in 0..30 {
            speaker.push_capture(&sine_frame(n));
            std::thread::sleep(Duration::from_millis(20));
            listener.render(&mut out);
            e += out.iter().map(|s| s * s).sum::<f32>();
        }
        speaker.set_push_to_talk(false);
        std::thread::sleep(Duration::from_millis(100));
        e
    };
    // Proximidad: no le afecta el volumen de grupo.
    listener.set_group_volume(0.0);
    assert!(heard() > 10.0);
    // Grupo: manda el volumen de grupo.
    server.create_group(&a, "Radio", None).unwrap();
    server.join_group(&b, "Radio", None).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(heard() < 0.01, "volumen de grupo a 0: no se oye");
    listener.set_group_volume(1.0);
    listener.set_volume(0.0);
    assert!(heard() > 10.0, "con el de proximidad a 0, el grupo se sigue oyendo");
}

/// Con varios hilos recibiendo, el mismo paquete repetido muchas veces a la vez solo
/// se acepta una: dos hilos pueden descifrarlo a la vez, pero solo uno lo marca.
#[test]
fn a_replayed_packet_is_relayed_once_with_several_receiving_threads() {
    use ferrovoz::protocol::{AudioUp, Environment, PacketType, TalkMode};
    let server = VoiceServer::start_with_receivers("127.0.0.1:0".parse().unwrap(), 48.0, 4).unwrap();
    let addr = server.local_addr().unwrap();
    let (speaker, listener) = ([71u8; 16], [72u8; 16]);
    let mut sockets = Vec::new();
    for (id, x) in [(speaker, 0.0), (listener, 2.0)] {
        let key = server.add_player(id);
        server.update_player(&id, [x, 64.0, 0.0], OVERWORLD);
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.send_to(&ferrovoz::protocol::seal(&key, PacketType::Hello, Some(&id), &[], 1), addr).unwrap();
        sockets.push((sock, key));
    }
    std::thread::sleep(Duration::from_millis(200));
    let (listener_sock, _) = &sockets[1];
    listener_sock.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let mut buf = [0u8; 1500];
    while listener_sock.recv(&mut buf).is_ok() {} // Pong y estados

    let (speaker_sock, key) = &sockets[0];
    let body = AudioUp { seq: 1, talk: TalkMode::Normal, env: Environment::default(), opus: vec![0xF8; 40] }.encode();
    let pkt = ferrovoz::protocol::seal(key, PacketType::AudioUp, Some(&speaker), &body, 2);
    for _ in 0..200 {
        speaker_sock.send_to(&pkt, addr).unwrap();
    }
    std::thread::sleep(Duration::from_millis(300));
    let mut voice = 0;
    while let Ok(n) = listener_sock.recv(&mut buf) {
        if n > 4 && buf[3] == PacketType::AudioDown as u8 {
            voice += 1;
        }
    }
    assert_eq!(voice, 1, "el paquete repetido solo se reenvía una vez");
    // Las demás las para el anti-replay o, antes, el límite de tráfico del jugador.
    let st = server.stats();
    let stopped = st.replayed.load(std::sync::atomic::Ordering::Relaxed) + st.rate_limited.load(std::sync::atomic::Ordering::Relaxed);
    assert!(stopped >= 150, "las copias se descartan: {stopped}");
}
