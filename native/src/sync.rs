//! Locks que sobreviven a un fallo.
//!
//! Si un hilo falla (panic) con un `Mutex` cogido, Rust lo marca como envenenado y
//! `lock().unwrap()` hace fallar también a todos los demás hilos que lo usen: un fallo
//! en una trama tiraría la voz de todo el servidor. Aquí se sigue con el estado tal
//! como quedó, que en el peor caso es una trama o un ajuste a medias.

use std::sync::{Mutex, MutexGuard, PoisonError};

pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
