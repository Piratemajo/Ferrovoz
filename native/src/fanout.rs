//! Fan-out de la voz en varios hilos.
//!
//! Con mucha gente junta, casi todo el trabajo del relé es cifrar y enviar la misma
//! trama a cada oyente. El hilo que recibe decide los destinos y reserva los
//! contadores bajo el lock del estado; la copia de cada oyente, el cifrado y el
//! envío se hacen fuera del lock, repartidos en el pool.
//!
//! El orden entre oyentes distintos da igual, y para un mismo oyente el jitter
//! buffer y la ventana anti-replay toleran desorden.

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::protocol::{self, AudioDown, PacketType, PlayerKey};

/// Paquete pendiente de cifrar para un oyente UDP.
pub struct SealJob {
    pub addr: SocketAddr,
    pub key: PlayerKey,
    pub counter: u64,
    pub body: Body,
}

/// Contenido de un [`SealJob`].
pub enum Body {
    Ready(Vec<u8>),
    /// Una trama de voz común a todos los oyentes y lo que cambia para este: se monta
    /// al cifrar, fuera del lock.
    Audio { template: Arc<Vec<u8>>, rel: [f32; 3], occlusion: u8 },
}

impl SealJob {
    fn seal(self) -> (SocketAddr, Vec<u8>) {
        let body = match self.body {
            Body::Ready(b) => b,
            Body::Audio { template, rel, occlusion } => AudioDown::for_listener(&template, rel, occlusion),
        };
        (self.addr, protocol::seal(&self.key, PacketType::AudioDown, None, &body, self.counter))
    }
}

/// Por debajo de esto no compensa repartir.
const MIN_PARALLEL: usize = 32;
/// Lotes en cola por hilo; si se llena, lo hace el hilo que recibe.
const QUEUE_PER_WORKER: usize = 8;

/// Cifra y envía en el hilo actual. Devuelve los paquetes enviados.
pub fn seal_and_send(socket: &UdpSocket, jobs: Vec<SealJob>) -> usize {
    if jobs.is_empty() {
        return 0;
    }
    let out: Vec<_> = jobs.into_iter().map(SealJob::seal).collect();
    crate::udpbatch::send_all(socket, &out)
}

/// Callback con (paquetes enviados, ns de trabajo de un hilo auxiliar; 0 si fue el principal).
type Counter = Arc<dyn Fn(usize, u64) + Send + Sync>;

pub struct Pool {
    socket: UdpSocket,
    tx: Option<SyncSender<Vec<SealJob>>>,
    threads: Vec<JoinHandle<()>>,
    on_sent: Counter,
}

impl Pool {
    /// Hilos auxiliares: los núcleos que no reciben, máximo 7. Con un núcleo, ninguno.
    pub fn default_workers(receivers: usize) -> usize {
        cores().saturating_sub(receivers).min(7)
    }

    pub fn new(socket: &UdpSocket, workers: usize, on_sent: Counter) -> std::io::Result<Self> {
        let mut pool = Self { socket: socket.try_clone()?, tx: None, threads: Vec::new(), on_sent };
        if workers == 0 {
            return Ok(pool);
        }
        let (tx, rx) = mpsc::sync_channel::<Vec<SealJob>>(workers * QUEUE_PER_WORKER);
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..workers {
            let socket = socket.try_clone()?;
            let rx = rx.clone();
            let on_sent = pool.on_sent.clone();
            let t = std::thread::Builder::new()
                .name(format!("ferrovoz-fanout-{i}"))
                .spawn(move || worker(&socket, &rx, &*on_sent))?;
            pool.threads.push(t);
        }
        pool.tx = Some(tx);
        Ok(pool)
    }

    pub fn workers(&self) -> usize {
        self.threads.len()
    }

    /// Cifra y envía `jobs`, parte aquí y parte en los auxiliares.
    pub fn dispatch(&self, mut jobs: Vec<SealJob>) {
        let Some(tx) = &self.tx else {
            (self.on_sent)(seal_and_send(&self.socket, jobs), 0);
            return;
        };
        if jobs.len() < MIN_PARALLEL {
            (self.on_sent)(seal_and_send(&self.socket, jobs), 0);
            return;
        }
        // Trozos iguales para cada hilo.
        let parts = self.threads.len() + 1;
        let chunk = jobs.len().div_ceil(parts);
        let mut mine = Vec::new();
        while jobs.len() > chunk {
            let rest = jobs.split_off(jobs.len() - chunk);
            match tx.try_send(rest) {
                Ok(()) => {}
                // Cola llena: se hace aquí, así nunca se acumula retraso.
                Err(TrySendError::Full(rest)) | Err(TrySendError::Disconnected(rest)) => mine.extend(rest),
            }
        }
        mine.extend(jobs);
        (self.on_sent)(seal_and_send(&self.socket, mine), 0);
    }
}

pub fn cores() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

fn worker(socket: &UdpSocket, rx: &Mutex<Receiver<Vec<SealJob>>>, on_sent: &(dyn Fn(usize, u64) + Send + Sync)) {
    loop {
        let batch = match rx.lock().unwrap().recv() {
            Ok(b) => b,
            Err(_) => return, // el relé se ha parado
        };
        let started = std::time::Instant::now();
        let sent = seal_and_send(socket, batch);
        on_sent(sent, started.elapsed().as_nanos() as u64);
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.tx = None; // cierra el canal y los hilos terminan
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Con varios hilos cada paquete llega una vez, a su oyente y con su contador.
    #[test]
    fn every_job_is_sealed_and_sent_once() {
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let listeners: Vec<UdpSocket> = (0..4).map(|_| UdpSocket::bind("127.0.0.1:0").unwrap()).collect();
        for l in &listeners {
            l.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        }
        let key = protocol::random_key();
        let sent = Arc::new(AtomicUsize::new(0));
        let counted = sent.clone();
        let pool = Pool::new(&sender, 3, Arc::new(move |n, _| {
            counted.fetch_add(n, Ordering::Relaxed);
        }))
        .unwrap();
        assert_eq!(pool.workers(), 3);

        // 50 paquetes por oyente en una tanda, repartidos entre 4 hilos.
        let mut jobs = Vec::new();
        for counter in 1..=50u64 {
            for l in &listeners {
                jobs.push(SealJob { addr: l.local_addr().unwrap(), key, counter, body: Body::Ready(counter.to_le_bytes().to_vec()) });
            }
        }
        pool.dispatch(jobs);

        let mut buf = [0u8; 2048];
        for l in &listeners {
            let mut seen = HashSet::new();
            for _ in 0..50 {
                let n = l.recv(&mut buf).expect("missing packet");
                let raw = protocol::parse(&buf[..n]).unwrap();
                let mut window = protocol::ReplayWindow::default();
                let body = raw.open_once(&key, &mut window).unwrap();
                assert!(seen.insert(u64::from_le_bytes(body[..8].try_into().unwrap())));
            }
            assert_eq!(seen.len(), 50);
        }
        drop(pool); // espera a los hilos
        assert_eq!(sent.load(Ordering::Relaxed), 200);
    }
}
