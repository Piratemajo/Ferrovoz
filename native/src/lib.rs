//! Ferrovoz: chat de voz por proximidad para Minecraft.
//!
//! Capa JNI. Cada `Java_dev_ferrovoz_FerrovozNative_*` corresponde a un
//! `static native` de `dev.ferrovoz.FerrovozNative`.
//!
//! Los objetos se pasan a Java como un `long` (identificador en un registro de
//! `Arc`, ver `into_handle`). Llamar con un handle ya parado no hace nada.

pub mod client;
pub mod forwarder;
pub mod codec;
pub mod channels;
pub mod password;
pub mod sync;
mod api_jni;
pub mod crowd;
pub mod denoise;
pub mod jitter;
pub mod protocol;
pub mod resample;
pub mod reverb;
pub mod server;
pub mod spatial;
pub mod udpbatch;
pub mod autogain;
pub mod fanout;
pub mod hrtf;
pub mod micproc;
pub mod roles;
pub mod underwater;

#[cfg(feature = "audio")]
mod audio;

use client::{TransmitMode, TransportPolicy, VoiceClient};
use protocol::TalkMode;
use jni::objects::{JByteArray, JClass, JFloatArray, JLongArray, JObject, JObjectArray, JString};
use jni::sys::{jboolean, jbyteArray, jfloat, jfloatArray, jint, jlong, jlongArray, jobjectArray, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use server::{Ranges, VoiceServer};
use std::net::{SocketAddr, ToSocketAddrs};
use std::any::Any;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------

/// Ejecuta `f` capturando cualquier panic para que no llegue a la JVM.
fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        eprintln!("[ferrovoz] panic in native code (caught)");
        default
    })
}

// Registro de handles. Java guarda un número; cada llamada JNI saca un `Arc` del
// registro y lo suelta al terminar, así que un `*Stop` desde otro hilo (túnel, red)
// no destruye el objeto mientras se usa. Un handle parado, repetido o de otro tipo
// da `None`.

type AnyHandle = Arc<dyn Any + Send + Sync>;

fn handles() -> &'static RwLock<HashMap<jlong, AnyHandle>> {
    static HANDLES: OnceLock<RwLock<HashMap<jlong, AnyHandle>>> = OnceLock::new();
    HANDLES.get_or_init(Default::default)
}

fn into_handle<T: Any + Send + Sync>(v: T) -> jlong {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    handles().write().unwrap_or_else(PoisonError::into_inner).insert(h, Arc::new(v));
    h
}

/// El objeto del handle mientras dure la llamada, o `None` si ya se paró.
fn borrow<T: Any + Send + Sync>(h: jlong) -> Option<Arc<T>> {
    if h == 0 {
        return None;
    }
    let any = handles().read().unwrap_or_else(PoisonError::into_inner).get(&h)?.clone();
    any.downcast::<T>().ok()
}

/// Quita el handle del registro y espera (2 s como mucho) a que acaben las llamadas
/// en curso, para que el puerto UDP y los hilos estén libres al volver a Java. Si
/// alguna tarda más, el objeto se destruye cuando termine.
fn take<T: Any + Send + Sync>(h: jlong) -> Option<Arc<T>> {
    if h == 0 {
        return None;
    }
    let mut map = handles().write().unwrap_or_else(PoisonError::into_inner);
    let any = map.get(&h)?.clone();
    let obj = any.downcast::<T>().ok()?;
    map.remove(&h);
    drop(map);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Arc::strong_count(&obj) > 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    Some(obj)
}

fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    let host = if host.is_empty() { "0.0.0.0" } else { host };
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    // IPv4 primero: el servidor escucha en 0.0.0.0 y "localhost" suele resolver a ::1.
    addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).copied()
}

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn jbool(b: bool) -> jboolean {
    if b { JNI_TRUE } else { JNI_FALSE }
}

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[ferrovoz/{}] {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

fn init_logging() {
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info));
}

// ---------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_version(env: JNIEnv, _: JClass) -> jstring {
    init_logging();
    let v = format!("{}{}", env!("CARGO_PKG_VERSION"), if cfg!(feature = "audio") { "+audio" } else { "" });
    env.new_string(v).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

// ---------------------------------------------------------------------------
// Servidor
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverStart(
    mut env: JNIEnv,
    _: JClass,
    bind_host: JString,
    port: jint,
    max_distance: jfloat,
) -> jlong {
    let host = jstr(&mut env, &bind_host);
    guard(0, || {
        let Some(addr) = resolve(&host, port as u16) else { return 0 };
        match VoiceServer::start(addr, max_distance) {
            Ok(s) => {
                log::info!("voice server listening on {}", s.local_addr().map(|a| a.to_string()).unwrap_or_default());
                into_handle(s)
            }
            Err(e) => {
                log::error!("could not open voice port {addr}: {e}");
                0
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverStop(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || drop(take::<VoiceServer>(h)))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverAddPlayer(
    env: JNIEnv,
    _: JClass,
    h: jlong,
    msb: jlong,
    lsb: jlong,
) -> jbyteArray {
    let key = guard(None, || borrow::<VoiceServer>(h).map(|s| s.add_player(protocol::uuid_from_java(msb, lsb))));
    match key {
        Some(k) => env.byte_array_from_slice(&k).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverRemovePlayer(_: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) {
    guard((), || {
        if let Some(s) = borrow::<VoiceServer>(h) {
            s.remove_player(&protocol::uuid_from_java(msb, lsb));
        }
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverUpdatePlayer(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    msb: jlong,
    lsb: jlong,
    x: f64,
    y: f64,
    z: f64,
    dimension: jint,
    underwater: jboolean,
    role: jint,
) {
    guard((), || {
        if let Some(s) = borrow::<VoiceServer>(h) {
            s.update_player_with_role(
                &protocol::uuid_from_java(msb, lsb),
                [x, y, z],
                dimension,
                underwater != 0,
                crate::roles::Role::from_i32(role),
            );
        }
    })
}

/// Posiciones de `count` jugadores en una llamada: `ids` = msb, lsb; `pos` = x, y, z;
/// `info` = dimensión, flags (bit 0 bajo el agua, bits 1-2 rol). Lo que no cuadre
/// con `count` se ignora entero.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverUpdatePlayers(
    env: JNIEnv,
    _: JClass,
    h: jlong,
    ids: jni::objects::JLongArray,
    pos: jni::objects::JDoubleArray,
    info: jni::objects::JIntArray,
    count: jint,
) {
    let Ok(n) = usize::try_from(count) else { return };
    if n == 0 || n > 100_000 {
        return;
    }
    fn len<T: jni::objects::TypeArray>(env: &JNIEnv, a: &jni::objects::JPrimitiveArray<T>) -> usize {
        env.get_array_length(a).map_or(0, |l| l.max(0) as usize)
    }
    if ids.is_null() || pos.is_null() || info.is_null() || len(&env, &ids) < n * 2 || len(&env, &pos) < n * 3 || len(&env, &info) < n * 2 {
        return;
    }
    let (mut raw_ids, mut raw_pos, mut raw_info) = (vec![0i64; n * 2], vec![0f64; n * 3], vec![0i32; n * 2]);
    if env.get_long_array_region(&ids, 0, &mut raw_ids).is_err()
        || env.get_double_array_region(&pos, 0, &mut raw_pos).is_err()
        || env.get_int_array_region(&info, 0, &mut raw_info).is_err()
    {
        return;
    }
    let updates: Vec<server::PlayerUpdate> = (0..n)
        .filter(|&i| raw_pos[i * 3..i * 3 + 3].iter().all(|v| v.is_finite()))
        .map(|i| server::PlayerUpdate {
            id: protocol::uuid_from_java(raw_ids[i * 2], raw_ids[i * 2 + 1]),
            pos: [raw_pos[i * 3], raw_pos[i * 3 + 1], raw_pos[i * 3 + 2]],
            dimension: raw_info[i * 2],
            underwater: raw_info[i * 2 + 1] & 1 != 0,
            role: crate::roles::Role::from_i32((raw_info[i * 2 + 1] >> 1) & 3),
        })
        .collect();
    guard((), || {
        if let Some(s) = borrow::<VoiceServer>(h) {
            s.update_players(&updates);
        }
    })
}

/// Reglas de espectadores y muertos: 0 normal, 1 listen, 2 ghost, 3 own, 4 off.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetRoleRules(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    spectator: jint,
    dead: jint,
) {
    guard((), || {
        if let Some(s) = borrow::<VoiceServer>(h) {
            s.set_role_rules(crate::roles::RoleRules {
                spectator: crate::roles::RoleRule::from_i32(spectator),
                dead: crate::roles::RoleRule::from_i32(dead),
            });
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetMuted(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    msb: jlong,
    lsb: jlong,
    muted: jboolean,
) {
    guard((), || {
        if let Some(s) = borrow::<VoiceServer>(h) {
            s.set_muted(&protocol::uuid_from_java(msb, lsb), muted != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetMaxDistance(_: JNIEnv, _: JClass, h: jlong, d: jfloat) {
    guard((), || {
        if let Some(s) = borrow::<VoiceServer>(h) {
            s.set_max_distance(d);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverConnectedCount(_: JNIEnv, _: JClass, h: jlong) -> jint {
    guard(0, || borrow::<VoiceServer>(h).map_or(0, |s| s.connected_count() as jint))
}

// ---------------------------------------------------------------------------
// Cliente
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientStart(
    mut env: JNIEnv,
    _: JClass,
    host: JString,
    port: jint,
    msb: jlong,
    lsb: jlong,
    key: JByteArray,
    max_distance: jfloat,
    transport: jint,
) -> jlong {
    init_logging();
    let host = jstr(&mut env, &host);
    let key = env.convert_byte_array(&key).unwrap_or_default();
    guard(0, || {
        let Ok(key) = <[u8; protocol::KEY_LEN]>::try_from(key.as_slice()) else {
            log::error!("voice key has the wrong length");
            return 0;
        };
        let Some(addr) = resolve(&host, port as u16) else {
            log::error!("could not resolve {host}:{port}");
            return 0;
        };
        let policy = TransportPolicy::from_i32(transport);
        match VoiceClient::start_with(addr, protocol::uuid_from_java(msb, lsb), key, true, policy) {
            Ok(c) => {
                c.set_max_distance(max_distance);
                log::info!("connecting to voice server {addr}");
                into_handle(c)
            }
            Err(e) => {
                log::error!("could not start the voice client: {e}");
                0
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientStop(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || drop(take::<VoiceClient>(h)))
}

/// Una vez por tick: orientación del oyente y estado de la tecla.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientTick(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    yaw: jfloat,
    pitch: jfloat,
    ptt: jboolean,
    talk_mode: jint,
) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_listener_yaw(yaw);
            c.set_listener_pitch(pitch);
            c.set_push_to_talk(ptt != 0);
            c.set_talk_mode(TalkMode::from_u8(talk_mode as u8));
        }
    })
}

/// 0 = pulsar para hablar, 1 = activación por voz.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetMode(_: JNIEnv, _: JClass, h: jlong, mode: jint) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_mode(if mode == 1 { TransmitMode::VoiceActivation } else { TransmitMode::PushToTalk });
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientGetMode(_: JNIEnv, _: JClass, h: jlong) -> jint {
    guard(0, || borrow::<VoiceClient>(h).map_or(0, |c| c.mode() as jint))
}

macro_rules! client_setter {
    ($jname:ident, $method:ident, bool) => {
        #[no_mangle]
        pub extern "system" fn $jname(_: JNIEnv, _: JClass, h: jlong, v: jboolean) {
            guard((), || {
                if let Some(c) = borrow::<VoiceClient>(h) {
                    c.$method(v != 0);
                }
            })
        }
    };
    ($jname:ident, $method:ident, float) => {
        #[no_mangle]
        pub extern "system" fn $jname(_: JNIEnv, _: JClass, h: jlong, v: jfloat) {
            guard((), || {
                if let Some(c) = borrow::<VoiceClient>(h) {
                    c.$method(v);
                }
            })
        }
    };
}

client_setter!(Java_dev_ferrovoz_FerrovozNative_clientSetMonitor, set_monitor, bool);
client_setter!(Java_dev_ferrovoz_FerrovozNative_clientSetNoiseSuppression, set_noise_suppression, bool);
client_setter!(Java_dev_ferrovoz_FerrovozNative_clientSetMuted, set_muted, bool);
client_setter!(Java_dev_ferrovoz_FerrovozNative_clientSetDeafened, set_deafened, bool);
client_setter!(Java_dev_ferrovoz_FerrovozNative_clientSetVolume, set_volume, float);
client_setter!(Java_dev_ferrovoz_FerrovozNative_clientSetMicGain, set_mic_gain, float);
client_setter!(Java_dev_ferrovoz_FerrovozNative_clientSetVadThreshold, set_vad_threshold, float);

/// Cliente sin red para la pantalla de ajustes. Usa las mismas funciones `client*`
/// y se para con `clientStop`.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_previewStart(_: JNIEnv, _: JClass) -> jlong {
    init_logging();
    guard(0, || match VoiceClient::start_local(true) {
        Ok(c) => into_handle(c),
        Err(e) => {
            log::error!("could not start the audio test: {e}");
            0
        }
    })
}

/// Cambia de dispositivo en caliente. Cadena vacía = predeterminado.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetDevices(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    input: JString,
    output: JString,
) {
    let input = jstr(&mut env, &input);
    let output = jstr(&mut env, &output);
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_devices(&input, &output);
        }
    })
}

/// 0 = abriendo, 1 = abiertos, 2 = error.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientAudioStatus(_: JNIEnv, _: JClass, h: jlong) -> jint {
    guard(0, || borrow::<VoiceClient>(h).map_or(0, |c| c.audio_status() as jint))
}

fn string_array(env: &mut JNIEnv, items: &[String]) -> jobjectArray {
    let Ok(arr) = env.new_object_array(items.len() as i32, "java/lang/String", JObject::null()) else {
        return std::ptr::null_mut();
    };
    for (i, s) in items.iter().enumerate() {
        let Ok(js) = env.new_string(s) else { return std::ptr::null_mut() };
        if env.set_object_array_element(&arr, i as i32, &js).is_err() {
            return std::ptr::null_mut();
        }
        let _ = env.delete_local_ref(js);
    }
    arr.into_raw()
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_listInputDevices(mut env: JNIEnv, _: JClass) -> jobjectArray {
    #[cfg(feature = "audio")]
    let names = guard(Vec::new(), audio::input_devices);
    #[cfg(not(feature = "audio"))]
    let names: Vec<String> = Vec::new();
    string_array(&mut env, &names)
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_listOutputDevices(mut env: JNIEnv, _: JClass) -> jobjectArray {
    #[cfg(feature = "audio")]
    let names = guard(Vec::new(), audio::output_devices);
    #[cfg(not(feature = "audio"))]
    let names: Vec<String> = Vec::new();
    string_array(&mut env, &names)
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientIsTalking(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    jbool(guard(false, || borrow::<VoiceClient>(h).is_some_and(|c| c.is_talking())))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientIsConnected(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    jbool(guard(false, || borrow::<VoiceClient>(h).is_some_and(|c| c.is_connected())))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientUnderwaterAllowed(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    guard(1, || borrow::<VoiceClient>(h).map_or(1, |c| c.underwater_allowed() as jboolean))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientCaveEchoAllowed(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    guard(1, || borrow::<VoiceClient>(h).map_or(1, |c| c.cave_echo_allowed() as jboolean))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetCaveEcho(_: JNIEnv, _: JClass, h: jlong, enabled: jboolean) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_cave_echo(enabled != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetUnderwaterEffect(_: JNIEnv, _: JClass, h: jlong, enabled: jboolean) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_underwater_effect(enabled != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientGoodbye(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.goodbye();
        }
    })
}

/// HRTF (true) o paneo estéreo (false).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetHrtf(_: JNIEnv, _: JClass, h: jlong, enabled: jboolean) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_hrtf(enabled != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetMicProcessing(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    echo_cancel: jboolean,
    auto_gain: jboolean,
) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_mic_processing(echo_cancel != 0, auto_gain != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetUnderwater(_: JNIEnv, _: JClass, h: jlong, underwater: jboolean) {
    guard((), || {
        if let Some(c) = borrow::<VoiceClient>(h) {
            c.set_underwater(underwater != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientMicSilent(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    guard(0, || borrow::<VoiceClient>(h).map_or(0, |c| c.mic_silent() as jboolean))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientMicLevel(_: JNIEnv, _: JClass, h: jlong) -> jfloat {
    guard(0.0, || borrow::<VoiceClient>(h).map_or(0.0, |c| c.mic_level()))
}

/// `[msb0, lsb0, grupo0, msb1, lsb1, grupo1, ...]` de quien está hablando
/// (grupo = 1 si es voz de grupo).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSpeakingPlayers(env: JNIEnv, _: JClass, h: jlong) -> jlongArray {
    let flat: Vec<jlong> = guard(Vec::new(), || {
        borrow::<VoiceClient>(h)
            .map(|c| {
                c.speaking_players()
                    .iter()
                    .flat_map(|(id, group)| {
                        let (m, l) = protocol::uuid_to_java(id);
                        [m, l, *group as jlong]
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    let Ok(arr) = env.new_long_array(flat.len() as i32) else { return std::ptr::null_mut() };
    if env.set_long_array_region(&arr, 0, &flat).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

// ---------------------------------------------------------------------------
// Volumen por jugador, oclusión, reverb, túnel, estadísticas
// ---------------------------------------------------------------------------

fn client(h: jlong) -> Option<Arc<VoiceClient>> {
    borrow::<VoiceClient>(h)
}

fn server(h: jlong) -> Option<Arc<VoiceServer>> {
    borrow::<VoiceServer>(h)
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetPlayerVolume(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, volume: jfloat,
) {
    guard((), || {
        if let Some(c) = client(h) {
            c.set_player_volume(protocol::uuid_from_java(msb, lsb), volume);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetOcclusion(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, amount: jfloat,
) {
    guard((), || {
        if let Some(c) = client(h) {
            c.set_occlusion(protocol::uuid_from_java(msb, lsb), amount);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetEnvironment(
    _: JNIEnv, _: JClass, h: jlong, wet: jfloat, room: jfloat,
) {
    guard((), || {
        if let Some(c) = client(h) {
            c.set_environment(wet, room);
        }
    })
}

/// Acústica de la sala: `wet`, `room`, `damp` y reflexiones tempranas en grupos de 5
/// `[dx, dy, dz, retardo s, ganancia]` (dirección hacia la pared, coordenadas del mundo).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetAcoustics(
    env: JNIEnv,
    _: JClass,
    h: jlong,
    wet: jfloat,
    room: jfloat,
    damp: jfloat,
    reflections: JFloatArray,
) {
    let len = env.get_array_length(&reflections).unwrap_or(0).max(0) as usize;
    let mut raw = vec![0f32; len];
    if len > 0 && env.get_float_array_region(&reflections, 0, &mut raw).is_err() {
        return;
    }
    guard((), || {
        if let Some(c) = client(h) {
            let refl = raw
                .chunks_exact(5)
                .filter(|r| r.iter().all(|v| v.is_finite()))
                .map(|r| crate::reverb::Reflection { dir: [r[0], r[1], r[2]], delay: r[3], gain: r[4].clamp(0.0, 1.0) })
                .collect();
            c.set_acoustics(wet, room, damp, refl);
        }
    })
}

/// Camino alternativo para la voz de un jugador: dirección del hueco (unitaria),
/// longitud en bloques y esquinas. `has = false` lo borra.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetSoundPath(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    msb: jlong,
    lsb: jlong,
    has: jboolean,
    dx: jfloat,
    dy: jfloat,
    dz: jfloat,
    length: jfloat,
    corners: jint,
) {
    guard((), || {
        if let Some(c) = client(h) {
            let path = (has != 0).then_some(crate::client::SoundPath {
                dir: [dx, dy, dz],
                length,
                corners: corners.max(0) as u32,
            });
            c.set_sound_path(protocol::uuid_from_java(msb, lsb), path);
        }
    })
}

/// `[rtt_ms, pérdida %, bitrate, túnel (0/1)]`.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientNetStats(env: JNIEnv, _: JClass, h: jlong) -> jfloatArray {
    let v = guard([0.0f32; 4], || {
        client(h).map_or([0.0; 4], |c| {
            let s = c.net_stats();
            [s.rtt_ms as f32, s.loss_percent as f32, s.bitrate as f32, s.tunnel as u8 as f32]
        })
    });
    let Ok(arr) = env.new_float_array(4) else { return std::ptr::null_mut() };
    if env.set_float_array_region(&arr, 0, &v).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

/// Paquete recibido por el túnel.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientHandleTunnel(env: JNIEnv, _: JClass, h: jlong, data: JByteArray) {
    let data = env.convert_byte_array(&data).unwrap_or_default();
    guard((), || {
        if let Some(c) = client(h) {
            c.handle_tunnel(&data);
        }
    })
}

fn byte_arrays(env: &mut JNIEnv, items: &[Vec<u8>]) -> jobjectArray {
    let Ok(arr) = env.new_object_array(items.len() as i32, "[B", JObject::null()) else {
        return std::ptr::null_mut();
    };
    for (i, b) in items.iter().enumerate() {
        let Ok(jb) = env.byte_array_from_slice(b) else { return std::ptr::null_mut() };
        if env.set_object_array_element(&arr, i as i32, &jb).is_err() {
            return std::ptr::null_mut();
        }
        let _ = env.delete_local_ref(jb);
    }
    arr.into_raw()
}

/// Paquetes pendientes de enviar por el túnel.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientPollTunnel(mut env: JNIEnv, _: JClass, h: jlong) -> jobjectArray {
    let items = guard(Vec::new(), || client(h).map(|c| c.poll_tunnel()).unwrap_or_default());
    byte_arrays(&mut env, &items)
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverHandleTunnel(
    env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, data: JByteArray,
) {
    let data = env.convert_byte_array(&data).unwrap_or_default();
    guard((), || {
        if let Some(s) = server(h) {
            s.handle_tunnel(&protocol::uuid_from_java(msb, lsb), &data);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverPollTunnel(
    mut env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || server(h).map(|s| s.poll_tunnel(&protocol::uuid_from_java(msb, lsb))).unwrap_or_default());
    byte_arrays(&mut env, &items)
}

// --- Servidor: alcances, permisos, moderación, grupos ---

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetRanges(
    _: JNIEnv, _: JClass, h: jlong, normal: jfloat, whisper: jfloat, shout: jfloat,
) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_ranges(Ranges { normal, whisper, shout });
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetPermissions(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, can_speak: jboolean, can_shout: jboolean,
) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_permissions(&protocol::uuid_from_java(msb, lsb), can_speak != 0, can_shout != 0);
        }
    })
}

/// Voz del anfitrión: siempre por separado, aunque haya multitud.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetPriority(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, priority: jboolean,
) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_priority(&protocol::uuid_from_java(msb, lsb), priority != 0);
        }
    })
}

/// Voces por separado como mucho en cada zona; las demás van al murmullo (0 = sin límite).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetUnderwaterEffect(_: JNIEnv, _: JClass, h: jlong, enabled: jboolean) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_underwater_effect(enabled != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetCaveEcho(_: JNIEnv, _: JClass, h: jlong, enabled: jboolean) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_cave_echo(enabled != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetMaxVoices(_: JNIEnv, _: JClass, h: jlong, max: jint) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_max_voices(max.max(0) as usize);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverIsMuted(_: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) -> jboolean {
    jbool(guard(false, || server(h).is_some_and(|s| s.is_muted(&protocol::uuid_from_java(msb, lsb)))))
}

/// `[conectados, por túnel, grupos, recibidos, enviados, rechazados, replay, limitados,
/// ns ocupado del relé, ns ocupado de los hilos que reciben]`.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverStatus(env: JNIEnv, _: JClass, h: jlong) -> jlongArray {
    use std::sync::atomic::Ordering::Relaxed;
    let v: [jlong; 10] = guard([0; 10], || {
        server(h).map_or([0; 10], |s| {
            let st = s.stats();
            [
                s.connected_count() as jlong,
                s.tunnel_count() as jlong,
                s.list_groups().len() as jlong,
                st.packets_in.load(Relaxed) as jlong,
                st.packets_out.load(Relaxed) as jlong,
                st.rejected.load(Relaxed) as jlong,
                st.replayed.load(Relaxed) as jlong,
                st.rate_limited.load(Relaxed) as jlong,
                st.busy_ns.load(Relaxed) as jlong,
                st.receive_busy_ns.load(Relaxed) as jlong,
            ]
        })
    });
    let Ok(arr) = env.new_long_array(v.len() as i32) else { return std::ptr::null_mut() };
    if env.set_long_array_region(&arr, 0, &v).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

/// Pérdida (%) en la subida de un jugador, o -1 si no está.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverLossPercent(_: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) -> jint {
    guard(-1, || server(h).and_then(|s| s.loss_percent(&protocol::uuid_from_java(msb, lsb))).map_or(-1, |l| l as jint))
}

fn opt_string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        None
    } else {
        Some(jstr(env, s))
    }
}

/// 0 = ok; 1 nombre no válido, 2 ya existe, 3 no existe, 4 contraseña incorrecta,
/// 5 jugador desconocido.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverCreateGroup(
    mut env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, name: JString, password: JString,
) -> jint {
    let name = jstr(&mut env, &name);
    let password = opt_string(&mut env, &password);
    guard(5, || {
        server(h).map_or(5, |s| match s.create_group(&protocol::uuid_from_java(msb, lsb), &name, password.as_deref()) {
            Ok(()) => 0,
            Err(e) => e.code(),
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverJoinGroup(
    mut env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, name: JString, password: JString,
) -> jint {
    let name = jstr(&mut env, &name);
    let password = opt_string(&mut env, &password);
    guard(5, || {
        server(h).map_or(5, |s| match s.join_group(&protocol::uuid_from_java(msb, lsb), &name, password.as_deref()) {
            Ok(()) => 0,
            Err(e) => e.code(),
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverLeaveGroup(_: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) -> jboolean {
    jbool(guard(false, || server(h).is_some_and(|s| s.leave_group(&protocol::uuid_from_java(msb, lsb)))))
}

/// Nombre del grupo del jugador, o null.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverGroupOf(env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) -> jstring {
    match guard(None, || server(h).and_then(|s| s.group_of(&protocol::uuid_from_java(msb, lsb)))) {
        Some(name) => env.new_string(name).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

/// `["nombre|miembros|candado(0/1)|permanente(0/1)", ...]`
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverListGroups(mut env: JNIEnv, _: JClass, h: jlong) -> jobjectArray {
    let items: Vec<String> = guard(Vec::new(), || {
        server(h)
            .map(|s| {
                s.list_groups()
                    .into_iter()
                    .map(|(n, m, p, fixed)| format!("{n}|{m}|{}|{}", p as u8, fixed as u8))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

// ---------------------------------------------------------------------------
// Grupos con dueño y permanentes, eventos, persistencia, oclusión del servidor
// ---------------------------------------------------------------------------

fn long_array(env: &JNIEnv, v: &[jlong]) -> jlongArray {
    let Ok(arr) = env.new_long_array(v.len() as i32) else { return std::ptr::null_mut() };
    if env.set_long_array_region(&arr, 0, v).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

fn group_result(r: Result<String, server::GroupError>) -> jint {
    match r {
        Ok(_) => 0,
        Err(e) => e.code(),
    }
}

/// Fichero de silencios y grupos permanentes (se carga al llamarlo).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetDataFile(mut env: JNIEnv, _: JClass, h: jlong, path: JString) -> jboolean {
    let path = jstr(&mut env, &path);
    jbool(guard(false, || {
        server(h).is_some_and(|s| match s.set_data_file(std::path::Path::new(&path)) {
            Ok(()) => true,
            Err(e) => {
                log::error!("could not read {path}: {e}");
                false
            }
        })
    }))
}

/// 0 = ok, 6 = no es el dueño, 7 = no está en ningún grupo.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverKickFromGroup(
    _: JNIEnv, _: JClass, h: jlong, req_msb: jlong, req_lsb: jlong, msb: jlong, lsb: jlong, admin: jboolean,
) -> jint {
    guard(5, || {
        server(h).map_or(5, |s| {
            group_result(s.kick_from_group(
                &protocol::uuid_from_java(req_msb, req_lsb),
                &protocol::uuid_from_java(msb, lsb),
                admin != 0,
            ))
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetGroupPersistent(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, persistent: jboolean, admin: jboolean,
) -> jint {
    guard(5, || {
        server(h).map_or(5, |s| {
            group_result(s.set_group_persistent(&protocol::uuid_from_java(msb, lsb), persistent != 0, admin != 0))
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverDeleteGroup(
    mut env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, name: JString, admin: jboolean,
) -> jint {
    let name = jstr(&mut env, &name);
    guard(5, || server(h).map_or(5, |s| group_result(s.delete_group(&protocol::uuid_from_java(msb, lsb), &name, admin != 0))))
}

/// `[msb, lsb]` del dueño del grupo, o un array vacío.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverGroupOwner(mut env: JNIEnv, _: JClass, h: jlong, name: JString) -> jlongArray {
    let name = jstr(&mut env, &name);
    let v: Vec<jlong> = guard(Vec::new(), || {
        server(h)
            .and_then(|s| s.group_owner(&name))
            .map(|id| {
                let (m, l) = protocol::uuid_to_java(&id);
                vec![m, l]
            })
            .unwrap_or_default()
    });
    long_array(&env, &v)
}

/// `[msb0, lsb0, msb1, lsb1, ...]` de los miembros conectados del grupo.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverGroupMembers(mut env: JNIEnv, _: JClass, h: jlong, name: JString) -> jlongArray {
    let name = jstr(&mut env, &name);
    let v: Vec<jlong> = guard(Vec::new(), || {
        server(h)
            .map(|s| {
                s.group_members(&name)
                    .iter()
                    .flat_map(|id| {
                        let (m, l) = protocol::uuid_to_java(id);
                        [m, l]
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    long_array(&env, &v)
}

/// `["tipo|uuid|detalle", ...]`, en orden (ver `server::EventKind`). El detalle va al
/// final porque puede llevar cualquier carácter.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverPollEvents(mut env: JNIEnv, _: JClass, h: jlong) -> jobjectArray {
    let items: Vec<String> = guard(Vec::new(), || {
        server(h)
            .map(|s| {
                s.poll_events()
                    .into_iter()
                    .map(|e| format!("{}|{}|{}", e.kind.name(), protocol::uuid_to_string(&e.player), e.detail))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

/// `[hablaMsb, hablaLsb, oyeMsb, oyeLsb, ...]`: pares de proximidad activos (para la oclusión).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverActivePairs(env: JNIEnv, _: JClass, h: jlong) -> jlongArray {
    let v: Vec<jlong> = guard(Vec::new(), || {
        server(h)
            .map(|s| {
                s.active_pairs()
                    .iter()
                    .flat_map(|(a, b)| {
                        let (am, al) = protocol::uuid_to_java(a);
                        let (bm, bl) = protocol::uuid_to_java(b);
                        [am, al, bm, bl]
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    long_array(&env, &v)
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetOcclusion(
    _: JNIEnv, _: JClass, h: jlong, s_msb: jlong, s_lsb: jlong, l_msb: jlong, l_lsb: jlong, amount: jfloat,
) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_occlusion(&protocol::uuid_from_java(s_msb, s_lsb), &protocol::uuid_from_java(l_msb, l_lsb), amount);
        }
    })
}

/// Varias oclusiones de golpe: `pairs` = [hablante msb, lsb, oyente msb, lsb, ...]
/// y `amounts` una por pareja.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetOcclusions(
    env: JNIEnv, _: JClass, h: jlong, pairs: JLongArray, amounts: JFloatArray,
) {
    guard((), || {
        let Some(s) = server(h) else { return };
        let n = env.get_array_length(&amounts).unwrap_or(0).max(0) as usize;
        if n == 0 || env.get_array_length(&pairs).unwrap_or(0).max(0) as usize != n * 4 {
            return;
        }
        let mut ids = vec![0 as jlong; n * 4];
        let mut values = vec![0 as jfloat; n];
        if env.get_long_array_region(&pairs, 0, &mut ids).is_err() || env.get_float_array_region(&amounts, 0, &mut values).is_err() {
            return;
        }
        let batch: Vec<(protocol::PlayerId, protocol::PlayerId, f32)> = ids
            .chunks_exact(4)
            .zip(values)
            .map(|(q, v)| (protocol::uuid_from_java(q[0], q[1]), protocol::uuid_from_java(q[2], q[3]), v))
            .collect();
        s.set_occlusions(&batch);
    })
}

/// Nombre del grupo actual, o null.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientGroupName(env: JNIEnv, _: JClass, h: jlong) -> jstring {
    // El grupo de red del proxy tiene prioridad.
    let name = guard(String::new(), || {
        client(h)
            .map(|c| {
                let net = c.net_group();
                if net.name.is_empty() { c.group_state().name } else { net.name }
            })
            .unwrap_or_default()
    });
    if name.is_empty() {
        return std::ptr::null_mut();
    }
    env.new_string(name).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// Bits: 1 = dueño, 2 = permanente, 4 = grupo de red.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientGroupFlags(_: JNIEnv, _: JClass, h: jlong) -> jint {
    guard(0, || {
        client(h).map_or(0, |c| {
            let net = c.net_group();
            if !net.name.is_empty() {
                return (net.owner as jint) | 4;
            }
            let g = c.group_state();
            (g.owner as jint) | ((g.persistent as jint) << 1)
        })
    })
}

/// Nombre de un miembro del grupo de red (puede estar en otro servidor), o null.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientMemberName(
    env: JNIEnv,
    _: JClass,
    h: jlong,
    msb: jlong,
    lsb: jlong,
) -> jstring {
    let id = protocol::uuid_from_java(msb, lsb);
    let name = guard(None, || {
        client(h).and_then(|c| c.net_group().members.into_iter().find(|(m, _)| *m == id).map(|(_, n)| n))
    });
    match name {
        Some(n) => env.new_string(n).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

/// Pide al servidor los grupos que hay. La respuesta tarda un viaje de ida y vuelta
/// y se lee con `clientGroupList`. Un servidor anterior a 0.13 no contesta.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientRequestGroups(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || {
        if let Some(c) = client(h) {
            c.request_groups();
        }
    })
}

/// `["nombre|miembros|candado(0/1)|permanente(0/1)|propio(0/1)", ...]`, como
/// `serverListGroups` más el último campo. Vacío si el servidor no la ha mandado.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientGroupList(mut env: JNIEnv, _: JClass, h: jlong) -> jobjectArray {
    let items: Vec<String> = guard(Vec::new(), || {
        client(h)
            .map(|c| {
                c.groups()
                    .0
                    .into_iter()
                    .map(|g| format!("{}|{}|{}|{}|{}", g.name, g.members, g.locked as u8, g.persistent as u8, g.mine as u8))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

/// Miembros del grupo propio: pares (mitad alta, mitad baja) de sus UUID. El nombre lo
/// pone el mod, de la lista de jugadores o con `clientMemberName`.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientGroupMembers(env: JNIEnv, _: JClass, h: jlong) -> jlongArray {
    let v: Vec<jlong> = guard(Vec::new(), || {
        client(h)
            .map(|c| {
                // El grupo de red del proxy tiene prioridad, como en `clientGroupName`
                // y `clientGroupFlags`: los tres hablan del mismo grupo.
                let net = c.net_group();
                let ids: Vec<protocol::PlayerId> = if net.name.is_empty() {
                    c.groups().1
                } else {
                    net.members.into_iter().map(|(id, _)| id).collect()
                };
                ids.iter()
                    .flat_map(|id| {
                        let (msb, lsb) = protocol::uuid_to_java(id);
                        [msb, lsb]
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    long_array(&env, &v)
}

// ---------------------------------------------------------------------------
// Reenviador para proxies (Velocity, BungeeCord)
// ---------------------------------------------------------------------------

use crate::forwarder::VoiceForwarder;

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_proxyStart(mut env: JNIEnv, _: JClass, bind_host: JString, port: jint) -> jlong {
    let host = jstr(&mut env, &bind_host);
    guard(0, || {
        let Some(addr) = resolve(&host, port as u16) else { return 0 };
        match VoiceForwarder::start(addr) {
            Ok(f) => into_handle(f),
            Err(e) => {
                log::error!("could not open the proxy voice port {addr}: {e}");
                0
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_proxyStop(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || drop(take::<VoiceForwarder>(h)))
}

/// El jugador está en el servidor con relé `host:port`. false si no se resuelve.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_proxyRoute(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    msb: jlong,
    lsb: jlong,
    key: JByteArray,
    host: JString,
    port: jint,
) -> jboolean {
    let host = jstr(&mut env, &host);
    let key = env.convert_byte_array(&key).unwrap_or_default();
    guard(JNI_FALSE, || {
        let Some(f) = borrow::<VoiceForwarder>(h) else { return JNI_FALSE };
        let Ok(key) = <[u8; 32]>::try_from(key.as_slice()) else { return JNI_FALSE };
        // Host vacío no vale aquí (sería escuchar en todas las interfaces).
        let Some(addr) = (if host.is_empty() { None } else { resolve(&host, port as u16) }) else { return JNI_FALSE };
        f.route(protocol::uuid_from_java(msb, lsb), key, addr);
        JNI_TRUE
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_proxyRemove(_: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) {
    guard((), || {
        if let Some(f) = borrow::<VoiceForwarder>(h) {
            f.remove(&protocol::uuid_from_java(msb, lsb));
        }
    })
}

/// Grupo de red `group`: nombre, dueño (UUID de voz; 0,0 = ninguno) y miembros
/// (`ids` = pares msb,lsb; `names` = nombres). Sin miembros se borra.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_proxySetNetGroup(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    group: jint,
    name: JString,
    owner_msb: jlong,
    owner_lsb: jlong,
    ids: JLongArray,
    names: JObjectArray,
) {
    let name = jstr(&mut env, &name);
    let len = env.get_array_length(&ids).unwrap_or(0).max(0) as usize;
    let mut raw = vec![0 as jlong; len];
    if env.get_long_array_region(&ids, 0, &mut raw).is_err() {
        return;
    }
    let mut members = Vec::with_capacity(len / 2);
    for (i, pair) in raw.chunks_exact(2).enumerate() {
        let member_name = env
            .get_object_array_element(&names, i as i32)
            .map(JString::from)
            .map(|s| jstr(&mut env, &s))
            .unwrap_or_default();
        members.push((protocol::uuid_from_java(pair[0], pair[1]), member_name));
    }
    guard((), || {
        if let Some(f) = borrow::<VoiceForwarder>(h) {
            let owner = (owner_msb != 0 || owner_lsb != 0).then(|| protocol::uuid_from_java(owner_msb, owner_lsb));
            f.set_net_group(group as u32, name, owner, members);
        }
    })
}

/// `[rutas, subidos, bajados, rechazados, replay, voz de grupo entregada]`.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_proxyStatus(env: JNIEnv, _: JClass, h: jlong) -> jlongArray {
    use std::sync::atomic::Ordering::Relaxed;
    let v: [jlong; 6] = guard([0; 6], || {
        borrow::<VoiceForwarder>(h).map_or([0; 6], |f| {
            let s = f.stats();
            [
                f.routes() as jlong,
                s.packets_up.load(Relaxed) as jlong,
                s.packets_down.load(Relaxed) as jlong,
                s.rejected.load(Relaxed) as jlong,
                s.replayed.load(Relaxed) as jlong,
                s.packets_relayed.load(Relaxed) as jlong,
            ]
        })
    });
    let Ok(arr) = env.new_long_array(v.len() as i32) else { return std::ptr::null_mut() };
    if env.set_long_array_region(&arr, 0, &v).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

// ---------------------------------------------------------------------------
// Repeticiones: Replay Mod y Flashback
// ---------------------------------------------------------------------------

/// Activa la grabación de tramas para la repetición.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetRecording(_: JNIEnv, _: JClass, h: jlong, on: jboolean) {
    guard((), || {
        if let Some(c) = client(h) {
            c.set_recording(on != 0);
        }
    })
}

/// Tramas grabadas desde la última llamada (cuerpos `AudioDown`, sin cifrar).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientPollRecorded(mut env: JNIEnv, _: JClass, h: jlong) -> jobjectArray {
    let items = guard(Vec::new(), || client(h).map(|c| c.poll_recorded()).unwrap_or_default());
    byte_arrays(&mut env, &items)
}

/// Mezclador para reproducir una repetición, sin red ni dispositivos. El audio sale
/// por Minecraft con `replayRender` para que Flashback lo incluya al exportar.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_replayStart(_: JNIEnv, _: JClass) -> jlong {
    init_logging();
    guard(0, || match VoiceClient::start_local(false) {
        Ok(c) => into_handle(c),
        Err(e) => {
            log::error!("could not start replay voice playback: {e}");
            0
        }
    })
}

/// Una trama grabada. false si no se reconoce el formato.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_replayFeed(env: JNIEnv, _: JClass, h: jlong, data: JByteArray) -> jboolean {
    let data = env.convert_byte_array(&data).unwrap_or_default();
    guard(JNI_FALSE, || {
        if client(h).is_some_and(|c| c.replay_feed(&data)) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    })
}

/// Posición del hablante relativa a la cámara.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_replaySetPosition(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    msb: jlong,
    lsb: jlong,
    x: jfloat,
    y: jfloat,
    z: jfloat,
) {
    guard((), || {
        if let Some(c) = client(h) {
            c.set_source_position(protocol::uuid_from_java(msb, lsb), [x, y, z]);
        }
    })
}

/// Mezcla `frames` muestras (estéreo intercalado, 48 kHz) para Minecraft.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_replayRender(env: JNIEnv, _: JClass, h: jlong, frames: jint) -> jfloatArray {
    let n = (frames.clamp(0, 48_000) as usize) * 2;
    let out = guard(Vec::new(), || {
        client(h).map_or(Vec::new(), |c| {
            let mut buf = vec![0f32; n];
            c.render(&mut buf);
            buf
        })
    });
    let Ok(arr) = env.new_float_array(out.len() as i32) else { return std::ptr::null_mut() };
    if env.set_float_array_region(&arr, 0, &out).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

// ---------------------------------------------------------------------------
// Indicadores y volumen de grupo
// ---------------------------------------------------------------------------

/// Volumen de la voz de grupo (1.0 = 100 %).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientSetGroupVolume(_: JNIEnv, _: JClass, h: jlong, v: jfloat) {
    guard((), || {
        if let Some(c) = client(h) {
            c.set_group_volume(v);
        }
    })
}

/// `[msb, lsb, flags, ...]` con el estado de voz de cada jugador (1 = silenciado,
/// 2 = ensordecido, 4 = conectado), o null si el servidor no lo envía.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_clientPeerStates(env: JNIEnv, _: JClass, h: jlong) -> jlongArray {
    let states = guard(None, || client(h).and_then(|c| c.peer_states()));
    let Some(states) = states else { return std::ptr::null_mut() };
    let mut v = Vec::with_capacity(states.len() * 3);
    for (id, flags) in states {
        let (msb, lsb) = protocol::uuid_to_java(&id);
        v.extend_from_slice(&[msb, lsb, flags as jlong]);
    }
    let Ok(arr) = env.new_long_array(v.len() as i32) else { return std::ptr::null_mut() };
    if env.set_long_array_region(&arr, 0, &v).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

#[cfg(test)]
mod handle_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn local() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    /// Parar el relé con otros hilos usándolo (túnel, red) y que luego el handle ya
    /// no devuelva nada.
    #[test]
    fn stop_while_other_threads_use_the_handle() {
        for _ in 0..20 {
            let h = into_handle(VoiceServer::start(local(), 48.0).unwrap());
            let go = Arc::new(AtomicBool::new(true));
            let workers: Vec<_> = (0..4)
                .map(|i| {
                    let go = go.clone();
                    std::thread::spawn(move || {
                        let id = [i as u8; 16];
                        let mut calls = 0u32;
                        while go.load(Ordering::Relaxed) {
                            if let Some(s) = server(h) {
                                s.add_player(id);
                                let _ = s.poll_tunnel(&id);
                                s.handle_tunnel(&id, &[1, 2, 3]);
                                calls += 1;
                            }
                        }
                        calls
                    })
                })
                .collect();
            std::thread::sleep(Duration::from_millis(5));
            drop(take::<VoiceServer>(h));
            assert!(server(h).is_none());
            go.store(false, Ordering::Relaxed);
            for w in workers {
                w.join().unwrap();
            }
        }
    }

    /// Tras `*Stop` el puerto queda libre para volver a arrancar en él.
    #[test]
    fn stop_releases_the_port() {
        let s = VoiceServer::start(local(), 48.0).unwrap();
        let addr = s.local_addr().unwrap();
        let h = into_handle(s);
        let user = server(h).unwrap(); // una llamada "en curso" que acaba pronto
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(user);
        });
        drop(take::<VoiceServer>(h));
        t.join().unwrap();
        VoiceServer::start(addr, 48.0).expect("port should be free after stop");
    }

    #[test]
    fn stale_wrong_type_and_double_stop_are_harmless() {
        let h = into_handle(VoiceServer::start(local(), 48.0).unwrap());
        assert!(client(h).is_none(), "a server handle is not a client");
        assert!(take::<VoiceServer>(h).is_some());
        assert!(take::<VoiceServer>(h).is_none(), "second stop does nothing");
        assert!(server(h).is_none());
        assert!(server(0).is_none());
    }
}
