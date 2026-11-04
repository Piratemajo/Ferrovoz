//! Prueba de carga del relé: 50-100 clientes falsos por UDP real.
//!
//! Cada cliente es un socket que hace lo mismo que el mod a nivel de red (Hello y
//! una trama cifrada cada 20 ms) sin micro ni Opus real: el servidor no decodifica,
//! así que le da igual. Mide CPU del hilo del relé, paquetes por segundo, voz
//! entregada y memoria.
//!
//! No corre con `cargo test` normal (tarda ~40 s):
//!
//! ```text
//! cargo test --release --test load -- --ignored --nocapture
//! ```

use ferrovoz::protocol::{self, AudioUp, Environment, PacketType, TalkMode};
use ferrovoz::server::VoiceServer;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(20);
const RANGE: f32 = 48.0;
/// ~24 kbps de Opus, como el mod con buena red.
const OPUS_BYTES: usize = 60;
const MEASURE: Duration = Duration::from_secs(5);

struct FakeClient {
    id: [u8; 16],
    socket: Arc<UdpSocket>,
    received: Arc<AtomicU64>,
    pos: [f64; 3],
    talking: bool,
}

struct Report {
    name: &'static str,
    players: usize,
    talkers: usize,
    cpu_percent: f64,
    in_per_s: f64,
    out_per_s: f64,
    delivered_percent: f64,
    us_per_packet: f64,
    rss_mb: Option<f64>,
}

/// Tiempo de CPU (s) del hilo del relé según /proc (solo Linux). Más exacto que
/// `busy_ns`, que también cuenta el tiempo esperando CPU con la máquina cargada.
fn relay_cpu_seconds() -> Option<f64> {
    for task in std::fs::read_dir("/proc/self/task").ok()? {
        let dir = task.ok()?.path();
        let comm = std::fs::read_to_string(dir.join("comm")).unwrap_or_default();
        if comm.trim() != "ferrovoz-server" {
            continue;
        }
        let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
        // Tras el ")" del nombre: utime y stime son los campos 14 y 15 (11 y 12 aquí).
        let rest = &stat[stat.rfind(')')? + 2..];
        let f: Vec<&str> = rest.split_whitespace().collect();
        let ticks: f64 = f.get(11)?.parse::<f64>().ok()? + f.get(12)?.parse::<f64>().ok()?;
        return Some(ticks / 100.0); // USER_HZ = 100 en Linux
    }
    None
}

/// Memoria residente del proceso (solo Linux).
fn rss_mb() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

fn run(name: &'static str, positions: Vec<[f64; 3]>, talking: impl Fn(usize) -> bool) -> Report {
    let stop = Arc::new(AtomicBool::new(false));

    // Primero los clientes, para que la memoria medida después sea la del servidor.
    let mut clients = Vec::new();
    for (i, pos) in positions.iter().enumerate() {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap());
        socket.set_nonblocking(true).unwrap();
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&(i as u64 + 1).to_le_bytes());
        id[15] = 0xF0;
        clients.push(FakeClient { id, socket, received: Arc::new(AtomicU64::new(0)), pos: *pos, talking: talking(i) });
    }
    // Pocos hilos que vacían muchos sockets cada 1 ms, para no quitarle CPU al relé.
    let mut receivers = Vec::new();
    for chunk in clients.chunks(25) {
        let socks: Vec<(Arc<UdpSocket>, Arc<AtomicU64>)> =
            chunk.iter().map(|c| (c.socket.clone(), c.received.clone())).collect();
        let stop = stop.clone();
        receivers.push(std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while !stop.load(Ordering::Relaxed) {
                for (sock, received) in &socks {
                    while let Ok(n) = sock.recv(&mut buf) {
                        // Cabecera: 'F' 'V' versión tipo. Solo cuenta la voz.
                        if n > 4 && buf[3] == PacketType::AudioDown as u8 {
                            received.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
    }
    std::thread::sleep(Duration::from_millis(100));
    let rss_before = rss_mb();

    let server = VoiceServer::start("127.0.0.1:0".parse().unwrap(), RANGE).unwrap();
    let addr = server.local_addr().unwrap();
    let mut keys = Vec::new();
    for c in &clients {
        keys.push(server.add_player(c.id));
        server.update_player(&c.id, c.pos, 1);
    }
    let mut counters = vec![1u64; clients.len()];
    for (i, c) in clients.iter().enumerate() {
        c.socket.send_to(&protocol::seal(&keys[i], PacketType::Hello, Some(&c.id), &[], counters[i]), addr).unwrap();
        counters[i] += 1;
    }
    std::thread::sleep(Duration::from_millis(300));

    // Oyentes esperados por hablante (misma regla que el servidor: distancia <= alcance).
    let listeners_of = |i: usize| {
        clients
            .iter()
            .enumerate()
            .filter(|(j, o)| {
                *j != i && {
                    let d = [o.pos[0] - clients[i].pos[0], o.pos[1] - clients[i].pos[1], o.pos[2] - clients[i].pos[2]];
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= RANGE as f64
                }
            })
            .count() as u64
    };
    let talkers: Vec<usize> = (0..clients.len()).filter(|&i| clients[i].talking).collect();
    let expected_per_frame: u64 = talkers.iter().map(|&i| listeners_of(i)).sum();

    // 1 s de calentamiento y después se mide.
    let mut seq = 0u32;
    let mut send_frame = |seq: &mut u32| {
        for &i in &talkers {
            let body = AudioUp { seq: *seq, talk: TalkMode::Normal, env: Environment::default(), opus: vec![0x5A; OPUS_BYTES] }
                .encode();
            let pkt = protocol::seal(&keys[i], PacketType::AudioUp, Some(&clients[i].id), &body, counters[i]);
            counters[i] += 1;
            let _ = clients[i].socket.send_to(&pkt, addr);
        }
        *seq += 1;
    };
    let warmup_end = Instant::now() + Duration::from_secs(1);
    let mut next = Instant::now();
    while Instant::now() < warmup_end {
        send_frame(&mut seq);
        next += FRAME;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    std::thread::sleep(Duration::from_millis(100));

    let st = server.stats();
    let busy0 = st.busy_ns.load(Ordering::Relaxed);
    let cpu0 = relay_cpu_seconds();
    let in0 = st.packets_in.load(Ordering::Relaxed);
    let out0 = st.packets_out.load(Ordering::Relaxed);
    let recv0: u64 = clients.iter().map(|c| c.received.load(Ordering::Relaxed)).sum();
    let t0 = Instant::now();
    let mut frames = 0u64;
    next = Instant::now();
    while t0.elapsed() < MEASURE {
        send_frame(&mut seq);
        frames += 1;
        next += FRAME;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    // Lo último puede estar todavía en camino.
    std::thread::sleep(Duration::from_millis(200));
    let wall = t0.elapsed().as_secs_f64() - 0.2;
    let busy = match (cpu0, relay_cpu_seconds()) {
        (Some(a), Some(b)) => b - a,
        _ => (st.busy_ns.load(Ordering::Relaxed) - busy0) as f64 / 1e9,
    };
    let pin = st.packets_in.load(Ordering::Relaxed) - in0;
    let pout = st.packets_out.load(Ordering::Relaxed) - out0;
    let recv: u64 = clients.iter().map(|c| c.received.load(Ordering::Relaxed)).sum::<u64>() - recv0;
    let rss_after = rss_mb();

    stop.store(true, Ordering::Relaxed);
    for r in receivers {
        r.join().unwrap();
    }
    drop(server);

    let expected = expected_per_frame * frames;
    Report {
        name,
        players: clients.len(),
        talkers: talkers.len(),
        cpu_percent: 100.0 * busy / wall,
        in_per_s: pin as f64 / wall,
        out_per_s: pout as f64 / wall,
        delivered_percent: if expected == 0 { 100.0 } else { 100.0 * recv as f64 / expected as f64 },
        us_per_packet: if pout == 0 { 0.0 } else { busy * 1e6 / (pin + pout) as f64 },
        rss_mb: rss_before.zip(rss_after).map(|(a, b)| (b - a).max(0.0)),
    }
}

/// `n` jugadores en grupos de 5 (a 3 bloques unos de otros), grupos a 300 bloques.
fn clusters(n: usize) -> Vec<[f64; 3]> {
    (0..n).map(|i| [(i / 5) as f64 * 300.0 + (i % 5) as f64 * 3.0, 64.0, 0.0]).collect()
}

/// `n` jugadores juntos (una plaza de 20x20).
fn crowd(n: usize) -> Vec<[f64; 3]> {
    (0..n).map(|i| [(i % 10) as f64 * 2.0, 64.0, (i / 10) as f64 * 2.0]).collect()
}

fn print(r: &Report) {
    println!(
        "{:<38} {:>4} {:>4} {:>7.1} % {:>8.0} {:>9.0} {:>7.1} % {:>6.2} µs {:>8}",
        r.name,
        r.players,
        r.talkers,
        r.cpu_percent,
        r.in_per_s,
        r.out_per_s,
        r.delivered_percent,
        r.us_per_packet,
        r.rss_mb.map_or("n/d".to_string(), |m| format!("{m:.1} MB")),
    );
}

#[test]
#[ignore = "prueba de carga: cargo test --release --test load -- --ignored --nocapture"]
fn relay_handles_100_players() {
    let reports = vec![
        // Servidor normal: gente repartida hablando a ratos.
        run("100 repartidos, 20 % hablando", clusters(100), |i| i % 5 == 0),
        // Evento: 50 juntos, 10 hablando.
        run("50 juntos, 10 hablando", crowd(50), |i| i % 5 == 0),
        // Mucho más de lo normal: 100 juntos, 25 hablando.
        run("100 juntos, 25 hablando", crowd(100), |i| i % 4 == 0),
        // Techo: 100 juntos hablando todos a la vez.
        run("100 juntos, todos hablando (extremo)", crowd(100), |_| true),
    ];

    println!();
    println!(
        "{:<38} {:>4} {:>4} {:>9} {:>8} {:>9} {:>9} {:>9} {:>8}",
        "escenario", "jug", "hab", "CPU relé", "in/s", "out/s", "entrega", "coste", "memoria"
    );
    for r in &reports {
        print(r);
    }

    // Capacidad estimada con el coste del escenario realista más pesado.
    let heavy = &reports[2];
    let packets_per_s_at_80 = 0.8 * 1e6 / heavy.us_per_packet.max(0.001);
    // Suponiendo un 20 % hablando y ~8 jugadores cerca de cada uno.
    let players = packets_per_s_at_80 / (0.2 * (1.0 + 8.0) * 50.0);
    println!();
    println!(
        "Coste medido: {:.2} µs por paquete (cifrar + enviar). Con el 80 % de un núcleo: ~{:.0} paquetes/s,",
        heavy.us_per_packet, packets_per_s_at_80
    );
    println!("es decir, ~{players:.0} jugadores si habla a la vez el 20 % y cada uno tiene ~8 jugadores cerca.");

    // Requisitos (los dos primeros son situaciones reales).
    for r in &reports[..3] {
        assert!(r.delivered_percent >= 95.0, "{}: solo llega el {:.1} % de la voz", r.name, r.delivered_percent);
    }
    assert!(reports[0].cpu_percent < 25.0, "100 jugadores repartidos no deberían costar ni un cuarto de núcleo");
}
