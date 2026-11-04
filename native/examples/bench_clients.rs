//! Clientes falsos para el benchmark en un servidor de Minecraft real
//! (ver `mod/src/main/java/dev/ferrovoz/Bench.java`).
//!
//! Con `-Dferrovoz.bench=N` el servidor escribe `ferrovoz-bench/keys.txt` con el
//! puerto y la clave de cada jugador virtual. Esto hace de sus clientes: Hello y,
//! los que hablan, una trama cifrada cada 20 ms. Cuenta la voz recibida entre
//! `measure_start` y `measure_end` y la guarda en `ferrovoz-bench/clients.properties`.
//!
//! ```text
//! cargo run --release --example bench_clients -- <carpeta-del-servidor>/ferrovoz-bench [host]
//! ```

use ferrovoz::protocol::{self, AudioUp, Environment, PacketType, PlayerId, PlayerKey, TalkMode};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(20);
const OPUS_BYTES: usize = 60; // ~24 kbps
const MAX_RUNTIME: Duration = Duration::from_secs(900);

struct Client {
    id: PlayerId,
    key: PlayerKey,
    talking: bool,
    socket: Arc<UdpSocket>,
    counter: u64,
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

fn wait_for(path: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while !path.exists() {
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    true
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("uso: bench_clients <carpeta ferrovoz-bench> [host]"));
    let host = args.next().unwrap_or_else(|| "127.0.0.1".into());

    let keys = dir.join("keys.txt");
    println!("esperando {} …", keys.display());
    assert!(wait_for(&keys, Duration::from_secs(600)), "el servidor no ha escrito las claves");
    let text = std::fs::read_to_string(&keys).unwrap();
    let mut lines = text.lines();
    let port: u16 = lines.next().unwrap().strip_prefix("port ").unwrap().trim().parse().unwrap();
    let addr: SocketAddr = (host.as_str(), port).to_socket_addrs().unwrap().next().unwrap();

    let mut clients = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let id: PlayerId = hex(f[0]).try_into().unwrap();
        let key: PlayerKey = hex(f[1]).try_into().unwrap();
        let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").unwrap());
        socket.set_nonblocking(true).unwrap();
        clients.push(Client { id, key, talking: f[2] == "1", socket, counter: 1 });
    }
    let talkers = clients.iter().filter(|c| c.talking).count();
    println!("{} clientes ({} hablan) contra {addr}", clients.len(), talkers);

    // Recepción: pocos hilos para muchos sockets.
    let stop = Arc::new(AtomicBool::new(false));
    let received = Arc::new(AtomicU64::new(0));
    let mut receivers = Vec::new();
    for chunk in clients.chunks(25) {
        let socks: Vec<Arc<UdpSocket>> = chunk.iter().map(|c| c.socket.clone()).collect();
        let (stop, received) = (stop.clone(), received.clone());
        receivers.push(std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while !stop.load(Ordering::Relaxed) {
                for s in &socks {
                    while let Ok(n) = s.recv(&mut buf) {
                        if n > 4 && buf[3] == PacketType::AudioDown as u8 {
                            received.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
    }

    // Hello con reintentos (el servidor puede tardar en procesarlo).
    for _ in 0..3 {
        for c in clients.iter_mut() {
            let _ = c.socket.send_to(&protocol::seal(&c.key, PacketType::Hello, Some(&c.id), &[], c.counter), addr);
            c.counter += 1;
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    let (start_file, end_file) = (dir.join("measure_start"), dir.join("measure_end"));
    let began = Instant::now();
    let mut seq = 0u32;
    let mut sent_total = 0u64;
    let mut window: Option<(u64, u64)> = None; // (recibidos, enviados) al empezar a medir
    let mut result: Option<(u64, u64)> = None;
    let mut deadline: Option<Instant> = None; // fin previsto de la medición
    let mut next = Instant::now();
    while began.elapsed() < MAX_RUNTIME {
        for c in clients.iter_mut().filter(|c| c.talking) {
            let body = AudioUp { seq, talk: TalkMode::Normal, env: Environment::default(), opus: vec![0x5A; OPUS_BYTES] }.encode();
            let _ = c.socket.send_to(&protocol::seal(&c.key, PacketType::AudioUp, Some(&c.id), &body, c.counter), addr);
            c.counter += 1;
            sent_total += 1;
        }
        seq = seq.wrapping_add(1);
        if seq.is_multiple_of(25) {
            if window.is_none() && start_file.exists() {
                window = Some((received.load(Ordering::Relaxed), sent_total));
                // Segundos que va a medir el servidor.
                let secs: u64 = std::fs::read_to_string(&start_file).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(60);
                deadline = Some(Instant::now() + Duration::from_secs(secs + 30));
                println!("midiendo {secs} s…");
            }
            if deadline.is_some_and(|d| Instant::now() > d) && !end_file.exists() {
                eprintln!(
                    "El servidor no ha terminado la medición a tiempo: ha dejado de hacer ticks o va muy lento. \
                     Si en su log sale «Server empty for 60 seconds, pausing», pon pause-when-empty-seconds=0 en server.properties."
                );
                std::process::exit(2);
            }
            if let (Some((r0, s0)), true) = (window, end_file.exists()) {
                std::thread::sleep(Duration::from_millis(200)); // lo que aún está en camino
                result = Some((received.load(Ordering::Relaxed) - r0, sent_total - s0));
                break;
            }
        }
        next += FRAME;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    stop.store(true, Ordering::Relaxed);
    for r in receivers {
        let _ = r.join();
    }

    let (recv, sent) = result.expect("el servidor no llegó a terminar la medición");
    let out = format!("clients={}\ntalkers={talkers}\nvoice_sent={sent}\nvoice_received={recv}\n", clients.len());
    std::fs::write(dir.join("clients.properties"), &out).unwrap();
    print!("{out}");
}
