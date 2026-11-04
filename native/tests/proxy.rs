//! Voz detrás de un proxy: los clientes solo conocen el puerto del reenviador y
//! cada servidor tiene su relé.

use ferrovoz::client::VoiceClient;
use ferrovoz::forwarder::VoiceForwarder;
use ferrovoz::protocol::{FRAME_SAMPLES, SAMPLE_RATE};
use ferrovoz::server::VoiceServer;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const WORLD: i32 = 1;

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(5), "timeout esperando: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sine(n: usize) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 300.0 * (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

/// Energía que oye `listener` mientras habla `speaker`.
fn speak(speaker: &VoiceClient, listener: &VoiceClient) -> f32 {
    let mut out = vec![0f32; FRAME_SAMPLES * 2];
    for _ in 0..200 {
        listener.render(&mut out); // lo que llegó antes
    }
    speaker.set_push_to_talk(true);
    let mut e = 0.0;
    for n in 0..30 {
        speaker.push_capture(&sine(n));
        std::thread::sleep(Duration::from_millis(20));
        listener.render(&mut out);
        e += out.iter().map(|s| s * s).sum::<f32>();
    }
    speaker.set_push_to_talk(false);
    std::thread::sleep(Duration::from_millis(100));
    for _ in 0..10 {
        listener.render(&mut out); // vaciar
    }
    e
}

/// Un jugador entra en un servidor: el relé le da clave, el plugin aprende la ruta y
/// el cliente se conecta al puerto del proxy.
fn join(server: &VoiceServer, fwd: &VoiceForwarder, id: [u8; 16], x: f64) -> VoiceClient {
    let key = server.add_player(id);
    server.update_player(&id, [x, 64.0, 0.0], WORLD);
    fwd.route(id, key, server.local_addr().unwrap());
    let c = VoiceClient::start(fwd.local_addr(), id, key, false).unwrap();
    c.set_noise_suppression(false);
    c
}

#[test]
fn voice_goes_through_the_proxy_and_follows_server_switches() {
    let a = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let b = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let fwd = VoiceForwarder::start("127.0.0.1:0".parse().unwrap()).unwrap();
    let (ana, bob) = ([1u8; 16], [2u8; 16]);

    // Los dos en el servidor A, a 3 bloques.
    let ca = join(&a, &fwd, ana, 0.0);
    let cb = join(&a, &fwd, bob, 3.0);
    wait_until("conectados a A por el proxy", || ca.is_connected() && cb.is_connected());
    let heard = speak(&ca, &cb);
    println!("en A, por el proxy: {heard:.1}");
    assert!(heard > 10.0, "Bob oye a Ana a través del reenviador");
    assert_eq!(fwd.routes(), 2);

    // Cambian al servidor B: nuevas claves, nuevas rutas, mismo puerto del proxy.
    a.remove_player(&ana);
    a.remove_player(&bob);
    drop((ca, cb));
    let up_a = a.stats().packets_in.load(Ordering::Relaxed);
    let ca = join(&b, &fwd, ana, 0.0);
    let cb = join(&b, &fwd, bob, 3.0);
    wait_until("conectados a B por el proxy", || ca.is_connected() && cb.is_connected());
    let heard = speak(&cb, &ca);
    println!("en B, por el proxy: {heard:.1}");
    assert!(heard > 10.0, "tras cambiar de servidor se siguen oyendo");
    assert_eq!(fwd.routes(), 2, "una ruta por jugador, no se acumulan");
    assert!(a.stats().packets_in.load(Ordering::Relaxed) - up_a < 5, "al servidor A ya no le llega su voz");

    // Sale del proxy.
    fwd.remove(&bob);
    wait_until("ruta borrada", || fwd.routes() == 1);
}

/// Proxy UDP mínimo que copia todo lo que manda el cliente (para el atacante).
struct Sniffer {
    addr: SocketAddr,
    captured: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
}

impl Sniffer {
    fn new(target: SocketAddr) -> Self {
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
        std::thread::spawn(move || {
            let mut client: Option<SocketAddr> = None;
            let mut buf = [0u8; 1500];
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(20) {
                if let Ok((n, from)) = sock.recv_from(&mut buf) {
                    if from == target {
                        if let Some(c) = client {
                            let _ = sock.send_to(&buf[..n], c);
                        }
                    } else {
                        client = Some(from);
                        cap.lock().unwrap().push(buf[..n].to_vec());
                        let _ = sock.send_to(&buf[..n], target);
                    }
                }
            }
        });
        Self { addr, captured }
    }
}

#[test]
fn nobody_can_hijack_another_players_voice_at_the_proxy() {
    let srv = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let fwd = VoiceForwarder::start("127.0.0.1:0".parse().unwrap()).unwrap();
    let (victim, friend) = ([7u8; 16], [8u8; 16]);
    let kv = srv.add_player(victim);
    srv.update_player(&victim, [0.0, 64.0, 0.0], WORLD);
    fwd.route(victim, kv, srv.local_addr().unwrap());
    // La víctima pasa por un sniffer que copia sus paquetes.
    let sniff = Sniffer::new(fwd.local_addr());
    let cv = VoiceClient::start(sniff.addr, victim, kv, false).unwrap();
    cv.set_noise_suppression(false);
    let cf = join(&srv, &fwd, friend, 3.0);
    wait_until("conectados", || cv.is_connected() && cf.is_connected());
    assert!(speak(&cv, &cf) > 10.0);

    // El atacante reenvía todo lo capturado desde otra dirección.
    let attacker = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let captured = sniff.captured.lock().unwrap().clone();
    let before = fwd.stats().replayed.load(Ordering::Relaxed);
    for p in &captured {
        attacker.send_to(p, fwd.local_addr()).unwrap();
    }
    // Y un paquete falso con el UUID de la víctima y un contador nuevo (no puede cifrarlo).
    let mut forged = captured[0].clone();
    forged[24..32].copy_from_slice(&(u64::MAX - 1).to_be_bytes()); // cabecera 4 + UUID 16 + dirección 4
    attacker.send_to(&forged, fwd.local_addr()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let blocked = fwd.stats().replayed.load(Ordering::Relaxed) - before;
    println!("paquetes repetidos bloqueados en el proxy: {blocked} de {}", captured.len());
    assert!(blocked as usize >= captured.len() - 1, "todo lo reenviado se reconoce como repetido");

    // La víctima sigue oyendo a su amigo y el atacante no recibe nada.
    attacker.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    assert!(speak(&cf, &cv) > 10.0, "la voz sigue llegando a la víctima");
    let mut buf = [0u8; 1500];
    assert!(attacker.recv(&mut buf).is_err(), "el atacante no ha desviado nada hacia él");
    assert!(fwd.stats().rejected.load(Ordering::Relaxed) >= 1, "el paquete falso no se puede descifrar: descartado");
}

/// Grupos de red: Ana (A) y Bob (B) en el mismo grupo se oyen; Carla, al lado de Ana
/// pero fuera del grupo, no la oye.
#[test]
fn network_groups_span_servers() {
    let a = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let b = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
    let fwd = VoiceForwarder::start("127.0.0.1:0".parse().unwrap()).unwrap();
    let (ana, bob, carla) = ([11u8; 16], [12u8; 16], [13u8; 16]);
    let ca = join(&a, &fwd, ana, 0.0);
    let cc = join(&a, &fwd, carla, 2.0);
    let cb = join(&b, &fwd, bob, 0.0);
    wait_until("conectados", || ca.is_connected() && cb.is_connected() && cc.is_connected());

    // Sin grupo de red: Bob no oye a Ana; Carla sí (está al lado).
    assert!(speak(&ca, &cb) < 1.0);
    assert!(speak(&ca, &cc) > 10.0);

    fwd.set_net_group(7, "Expedición".into(), Some(ana), vec![(ana, "Ana".into()), (bob, "Bob".into())]);
    wait_until("los dos saben su grupo", || ca.net_group().name == "Expedición" && cb.net_group().name == "Expedición");
    assert!(ca.net_group().owner && !cb.net_group().owner);
    assert_eq!(cb.net_group().members.len(), 2);

    let bob_hears = speak(&ca, &cb);
    let carla_hears = speak(&ca, &cc);
    let ana_hears = speak(&cb, &ca);
    println!("grupo de red: Bob oye {bob_hears:.1}, Ana oye {ana_hears:.1}, Carla oye {carla_hears:.1}");
    assert!(bob_hears > 10.0, "Bob oye a Ana desde otro servidor");
    assert!(ana_hears > 10.0, "y Ana a Bob");
    assert!(carla_hears < 1.0, "lo del grupo es privado: Carla, al lado, no lo oye");
    assert!(fwd.stats().packets_relayed.load(Ordering::Relaxed) > 0);

    // Bob sale del grupo y deja de oír a Ana.
    fwd.set_net_group(7, "Expedición".into(), Some(ana), vec![(ana, "Ana".into())]);
    wait_until("Bob ya no está", || cb.net_group().name.is_empty());
    assert!(speak(&ca, &cb) < 1.0);
    // Sin miembros el grupo desaparece y Ana vuelve a proximidad.
    fwd.set_net_group(7, String::new(), None, vec![]);
    wait_until("Ana sin grupo", || ca.net_group().name.is_empty());
    assert!(speak(&ca, &cc) > 10.0);
}
