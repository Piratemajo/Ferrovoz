//! JNI de la API para plugins (`dev.ferrovoz.api`): canales de audio, escucha del
//! micro, estado de cada jugador y un códec Opus para que los plugins codifiquen y
//! decodifiquen sin otra librería.

use crate::channels::Target;
use crate::codec::{VoiceDecoder, VoiceEncoder};
use crate::protocol::{self, PlayerId, FRAME_SAMPLES};
use crate::{borrow, guard, into_handle, jbool, jstr, long_array, server, take};
use jni::objects::{JByteArray, JClass, JLongArray, JShortArray, JString};
use jni::sys::{jboolean, jbyteArray, jdouble, jfloat, jint, jlong, jlongArray, jshortArray};
use jni::JNIEnv;
use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

fn id(msb: jlong, lsb: jlong) -> PlayerId {
    protocol::uuid_from_java(msb, lsb)
}

fn ids_from(env: &JNIEnv, arr: &JLongArray) -> Vec<PlayerId> {
    if arr.is_null() {
        return Vec::new();
    }
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut raw = vec![0i64; len];
    if len > 0 && env.get_long_array_region(arr, 0, &mut raw).is_err() {
        return Vec::new();
    }
    raw.chunks_exact(2).map(|p| id(p[0], p[1])).collect()
}

fn ids_to(env: &JNIEnv, ids: &[PlayerId]) -> jlongArray {
    let v: Vec<jlong> = ids
        .iter()
        .flat_map(|i| {
            let (m, l) = protocol::uuid_to_java(i);
            [m, l]
        })
        .collect();
    long_array(env, &v)
}

fn pcm_from(env: &JNIEnv, arr: &JShortArray, gain: f32) -> Option<Vec<f32>> {
    if arr.is_null() || env.get_array_length(arr).ok()? as usize != FRAME_SAMPLES {
        return None;
    }
    let mut raw = vec![0i16; FRAME_SAMPLES];
    env.get_short_array_region(arr, 0, &mut raw).ok()?;
    let g = if gain.is_finite() { gain.clamp(0.0, 4.0) } else { 1.0 } / 32768.0;
    Some(raw.iter().map(|&s| (s as f32 * g).clamp(-1.0, 1.0)).collect())
}

fn pcm_to(env: &JNIEnv, pcm: &[f32; FRAME_SAMPLES]) -> jshortArray {
    let raw: Vec<i16> = pcm.iter().map(|&s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect();
    let Ok(arr) = env.new_short_array(FRAME_SAMPLES as i32) else { return std::ptr::null_mut() };
    if env.set_short_array_region(&arr, 0, &raw).is_err() {
        return std::ptr::null_mut();
    }
    arr.into_raw()
}

// ---------------------------------------------------------------------------
// Jugadores y servidor
// ---------------------------------------------------------------------------

/// Bits del estado de un jugador en el relé (ver `serverPlayerState`).
const STATE_REGISTERED: jint = 1;
const STATE_CONNECTED: jint = 2;
const STATE_SPEAKING: jint = 4;
const STATE_SELF_MUTED: jint = 8;
const STATE_DEAFENED: jint = 16;

/// Estado en bits: registrado 1, conectado 2, hablando 4, micro silenciado por él 8,
/// ensordecido 16. 0 si no está.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverPlayerState(_: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) -> jint {
    guard(0, || {
        let Some(s) = server(h) else { return 0 };
        let p = id(msb, lsb);
        let Some(flags) = s.self_flags(&p) else { return 0 };
        let mut v = STATE_REGISTERED;
        if s.is_connected(&p) {
            v |= STATE_CONNECTED;
        }
        if s.is_speaking(&p) {
            v |= STATE_SPEAKING;
        }
        if flags & protocol::PeerStates::MUTED != 0 {
            v |= STATE_SELF_MUTED;
        }
        if flags & protocol::PeerStates::DEAFENED != 0 {
            v |= STATE_DEAFENED;
        }
        v
    })
}

/// `[msb, lsb, ...]` de los jugadores registrados en el relé.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverPlayers(env: JNIEnv, _: JClass, h: jlong) -> jlongArray {
    let v = guard(Vec::new(), || server(h).map(|s| s.player_ids()).unwrap_or_default());
    ids_to(&env, &v)
}

/// Alcance propio (megáfono, escenario). `range <= 0` lo quita.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetRangeOverride(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, range: jfloat,
) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_range_override(&id(msb, lsb), (range > 0.0).then_some(range));
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetHush(_: JNIEnv, _: JClass, h: jlong, hush: jboolean) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_hush(hush != 0);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverIsHushed(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    jbool(guard(false, || server(h).is_some_and(|s| s.is_hushed())))
}

// ---------------------------------------------------------------------------
// Grupos sin pasar por los permisos de un jugador
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverForceJoinGroup(
    mut env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, name: JString,
) -> jint {
    let name = jstr(&mut env, &name);
    guard(5, || {
        server(h).map_or(5, |s| match s.force_join_group(&id(msb, lsb), &name) {
            Ok(()) => 0,
            Err(e) => e.code(),
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetGroupPersistentByName(
    mut env: JNIEnv, _: JClass, h: jlong, name: JString, persistent: jboolean,
) -> jint {
    let name = jstr(&mut env, &name);
    guard(2, || {
        server(h).map_or(2, |s| match s.set_group_persistent_by_name(&name, persistent != 0) {
            Ok(_) => 0,
            Err(e) => e.code(),
        })
    })
}

/// `[dueñoMsb, dueñoLsb, flags]` con flags: tiene dueño 1, contraseña 2, permanente 4.
/// null si no existe.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverGroupInfo(mut env: JNIEnv, _: JClass, h: jlong, name: JString) -> jlongArray {
    let name = jstr(&mut env, &name);
    let info = guard(None, || server(h).and_then(|s| s.group_info(&name)));
    let Some((owner, password, persistent)) = info else { return std::ptr::null_mut() };
    let (m, l) = owner.as_ref().map(protocol::uuid_to_java).unwrap_or((0, 0));
    let flags = owner.is_some() as jlong | (password as jlong) << 1 | (persistent as jlong) << 2;
    long_array(&env, &[m, l, flags])
}

// ---------------------------------------------------------------------------
// Escucha del micro
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverSetMicTap(_: JNIEnv, _: JClass, h: jlong, on: jboolean) {
    guard((), || {
        if let Some(s) = server(h) {
            s.set_mic_tap(on != 0);
        }
    })
}

/// Tramas del micro pendientes, seguidas en un solo array (little endian):
/// `uuid(16) seq(4) modo(1) flags(1) largo(2) opus(largo)`, con modo 0 normal,
/// 1 susurro, 2 grito y flags 1 = grupo. null si no hay ninguna.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverPollMicrophone(env: JNIEnv, _: JClass, h: jlong) -> jbyteArray {
    let frames = guard(Vec::new(), || server(h).map(|s| s.poll_mic()).unwrap_or_default());
    if frames.is_empty() {
        return std::ptr::null_mut();
    }
    let mut buf = Vec::with_capacity(frames.iter().map(|f| 24 + f.opus.len()).sum());
    for f in frames {
        buf.extend_from_slice(&f.player);
        buf.extend_from_slice(&f.seq.to_le_bytes());
        buf.push(f.talk as u8);
        buf.push(f.group as u8);
        buf.extend_from_slice(&(f.opus.len() as u16).to_le_bytes());
        buf.extend_from_slice(&f.opus);
    }
    env.byte_array_from_slice(&buf).map_or(std::ptr::null_mut(), |a| a.into_raw())
}

// ---------------------------------------------------------------------------
// Canales de audio
// ---------------------------------------------------------------------------

/// Canal nuevo, sin oyentes hasta que se le da un destino. `[msb, lsb]` o null.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelCreate(
    env: JNIEnv, _: JClass, h: jlong, range: jfloat, bitrate: jint, music: jboolean,
) -> jlongArray {
    let created = guard(None, || {
        server(h).and_then(|s| s.create_channel(Target::Listeners(HashSet::new()), range, bitrate, music != 0))
    });
    match created {
        Some(c) => ids_to(&env, &[c]),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelPoint(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, x: jdouble, y: jdouble, z: jdouble, dimension: jint,
) -> jboolean {
    if ![x, y, z].iter().all(|v| v.is_finite()) {
        return jbool(false);
    }
    jbool(guard(false, || {
        server(h).is_some_and(|s| s.set_channel_target(&id(msb, lsb), Target::Point { pos: [x, y, z], dim: dimension }))
    }))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelFollow(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, pmsb: jlong, plsb: jlong,
) -> jboolean {
    jbool(guard(false, || server(h).is_some_and(|s| s.set_channel_target(&id(msb, lsb), Target::Follow(id(pmsb, plsb))))))
}

/// Sin posición, a estos jugadores (`[msb, lsb, ...]`).
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelListeners(
    env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, players: JLongArray,
) -> jboolean {
    let set: HashSet<PlayerId> = ids_from(&env, &players).into_iter().collect();
    jbool(guard(false, || server(h).is_some_and(|s| s.set_channel_target(&id(msb, lsb), Target::Listeners(set)))))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelGroup(
    mut env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, group: JString,
) -> jboolean {
    let group = jstr(&mut env, &group);
    jbool(guard(false, || server(h).is_some_and(|s| s.set_channel_target(&id(msb, lsb), Target::Group(group)))))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelRange(
    _: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, range: jfloat,
) -> jboolean {
    jbool(guard(false, || server(h).is_some_and(|s| s.set_channel_range(&id(msb, lsb), range))))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelRemove(_: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong) {
    guard((), || {
        if let Some(s) = server(h) {
            s.remove_channel(&id(msb, lsb));
        }
    })
}

/// 960 muestras de 16 bits (20 ms a 48 kHz, mono) por `gain` (0..4). false si el
/// canal no existe o la trama no es de 960.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelSendPcm(
    env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, pcm: JShortArray, gain: jfloat,
) -> jboolean {
    let Some(pcm) = pcm_from(&env, &pcm, gain) else { return jbool(false) };
    jbool(guard(false, || server(h).is_some_and(|s| s.channel_send_pcm(&id(msb, lsb), &pcm))))
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_serverChannelSendOpus(
    env: JNIEnv, _: JClass, h: jlong, msb: jlong, lsb: jlong, opus: JByteArray,
) -> jboolean {
    let Ok(opus) = env.convert_byte_array(&opus) else { return jbool(false) };
    jbool(guard(false, || server(h).is_some_and(|s| s.channel_send_opus(&id(msb, lsb), &opus))))
}

// ---------------------------------------------------------------------------
// Códec Opus para plugins
// ---------------------------------------------------------------------------

/// Codificador de 20 ms a 48 kHz mono. 0 si Opus falla.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_opusEncoderCreate(_: JNIEnv, _: JClass, bitrate: jint, music: jboolean) -> jlong {
    guard(0, || {
        VoiceEncoder::with_application(bitrate.clamp(6_000, 256_000), music != 0).map_or(0, |e| into_handle(Mutex::new(e)))
    })
}

/// Trama Opus, o null si la entrada no es de 960 muestras.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_opusEncode(env: JNIEnv, _: JClass, h: jlong, pcm: JShortArray) -> jbyteArray {
    let Some(pcm) = pcm_from(&env, &pcm, 1.0) else { return std::ptr::null_mut() };
    let packet = guard(None, || {
        let enc = borrow::<Mutex<VoiceEncoder>>(h)?;
        let mut enc = enc.lock().unwrap_or_else(PoisonError::into_inner);
        enc.encode(&pcm).ok().map(<[u8]>::to_vec)
    });
    match packet {
        Some(p) => env.byte_array_from_slice(&p).map_or(std::ptr::null_mut(), |a| a.into_raw()),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_opusEncoderClose(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || {
        take::<Mutex<VoiceEncoder>>(h);
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_opusDecoderCreate(_: JNIEnv, _: JClass) -> jlong {
    guard(0, || VoiceDecoder::new().map_or(0, |d| into_handle(Mutex::new(d))))
}

/// 960 muestras de 16 bits. Con `packet` null (trama perdida) se rellena el hueco.
#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_opusDecode(env: JNIEnv, _: JClass, h: jlong, packet: JByteArray) -> jshortArray {
    let packet = if packet.is_null() { None } else { env.convert_byte_array(&packet).ok() };
    let pcm = guard(None, || {
        let dec = borrow::<Mutex<VoiceDecoder>>(h)?;
        let mut dec = dec.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out = [0f32; FRAME_SAMPLES];
        match &packet {
            Some(p) if !p.is_empty() => dec.decode(p, &mut out),
            _ => dec.conceal(&mut out),
        };
        Some(out)
    });
    match pcm {
        Some(p) => pcm_to(&env, &p),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_ferrovoz_FerrovozNative_opusDecoderClose(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || {
        take::<Mutex<VoiceDecoder>>(h);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_bits_do_not_overlap() {
        let all = [STATE_REGISTERED, STATE_CONNECTED, STATE_SPEAKING, STATE_SELF_MUTED, STATE_DEAFENED];
        assert_eq!(all.iter().fold(0, |a, b| a | b), 31);
    }
}
