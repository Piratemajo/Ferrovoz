//! Micro y altavoces con cpal (WASAPI, CoreAudio, ALSA/PipeWire).
//!
//! Los `cpal::Stream` no son `Send` en todas las plataformas, así que viven
//! en un hilo propio que los mantiene abiertos mientras el cliente funcione.

use crate::client::Shared;
use crate::protocol::{FRAME_SAMPLES, SAMPLE_RATE};
use crate::resample::LinearResampler;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub(crate) fn spawn(shared: Arc<Shared>) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().name("ferrovoz-audio".into()).spawn(move || {
        let host = cpal::default_host();
        while shared.is_running() {
            // (Re)abrir con la selección actual.
            let generation = shared.device_generation();
            let devices = shared.devices();
            let input = open_input(&host, &devices.input, shared.clone())
                .map_err(|e| log::error!("could not open microphone '{}': {e}", shown(&devices.input)))
                .ok();
            let output = open_output(&host, &devices.output, shared.clone())
                .map_err(|e| log::error!("could not open audio output '{}': {e}", shown(&devices.output)))
                .ok();
            let ok = input.is_some() && output.is_some();
            shared.audio_status.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
            if ok {
                log::info!("audio: microphone '{}', output '{}'", shown(&devices.input), shown(&devices.output));
            }

            // Abiertos hasta que cambie la selección o se pare el cliente.
            while shared.is_running() && shared.device_generation() == generation {
                std::thread::sleep(Duration::from_millis(100));
            }
            drop(input);
            drop(output);
        }
    })
}

fn shown(name: &str) -> &str {
    if name.is_empty() { "predeterminado" } else { name }
}

/// Micros disponibles.
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

/// Salidas disponibles.
pub fn output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

/// Dispositivo por nombre; si ya no existe, el predeterminado.
fn pick(
    name: &str,
    mut all: impl Iterator<Item = cpal::Device>,
    default: Option<cpal::Device>,
) -> Option<cpal::Device> {
    if !name.is_empty() {
        if let Some(d) = all.find(|d| d.name().map(|n| n == name).unwrap_or(false)) {
            return Some(d);
        }
        log::warn!("device '{name}' not found, using the system default");
    }
    default
}

type AnyError = Box<dyn std::error::Error>;

fn open_input(host: &cpal::Host, name: &str, shared: Arc<Shared>) -> Result<cpal::Stream, AnyError> {
    let device = pick(name, host.input_devices()?, host.default_input_device()).ok_or("no hay micrófono")?;
    let supported = device.default_input_config()?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => build_input::<f32>(&device, &config, shared)?,
        SampleFormat::I16 => build_input::<i16>(&device, &config, shared)?,
        SampleFormat::U16 => build_input::<u16>(&device, &config, shared)?,
        SampleFormat::I32 => build_input::<i32>(&device, &config, shared)?,
        other => return Err(format!("formato de micrófono no soportado: {other:?}").into()),
    };
    stream.play()?;
    Ok(stream)
}

fn build_input<T>(device: &cpal::Device, config: &StreamConfig, shared: Arc<Shared>) -> Result<cpal::Stream, AnyError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let mut resampler = LinearResampler::new(config.sample_rate.0, SAMPLE_RATE);
    let mut mono = Vec::with_capacity(4096);
    let mut resampled = Vec::with_capacity(4096);
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            mono.clear();
            // Downmix a mono.
            for frame in data.chunks(channels) {
                let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
                mono.push(sum / channels as f32);
            }
            resampled.clear();
            resampler.process(&mono, &mut resampled);
            shared.push_capture(&resampled);
        },
        |e| log::error!("microphone error: {e}"),
        None,
    )?;
    Ok(stream)
}

fn open_output(host: &cpal::Host, name: &str, shared: Arc<Shared>) -> Result<cpal::Stream, AnyError> {
    let device = pick(name, host.output_devices()?, host.default_output_device()).ok_or("no hay salida de audio")?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => build_output::<f32>(&device, &config, shared)?,
        SampleFormat::I16 => build_output::<i16>(&device, &config, shared)?,
        SampleFormat::U16 => build_output::<u16>(&device, &config, shared)?,
        SampleFormat::I32 => build_output::<i32>(&device, &config, shared)?,
        other => return Err(format!("formato de salida no soportado: {other:?}").into()),
    };
    stream.play()?;
    Ok(stream)
}

fn build_output<T>(device: &cpal::Device, config: &StreamConfig, shared: Arc<Shared>) -> Result<cpal::Stream, AnyError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut res_l = LinearResampler::new(SAMPLE_RATE, config.sample_rate.0);
    let mut res_r = LinearResampler::new(SAMPLE_RATE, config.sample_rate.0);
    let mut stereo = vec![0f32; FRAME_SAMPLES * 2];
    let (mut in_l, mut in_r) = (Vec::with_capacity(FRAME_SAMPLES), Vec::with_capacity(FRAME_SAMPLES));
    let (mut out_l, mut out_r) = (Vec::with_capacity(2048), Vec::with_capacity(2048));
    let mut queue: VecDeque<(f32, f32)> = VecDeque::with_capacity(4096);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels;
            // 20 ms de mezcla a 48 kHz cada vez que haga falta, remuestreados a la
            // frecuencia del dispositivo.
            while queue.len() < frames {
                shared.render(&mut stereo);
                in_l.clear();
                in_r.clear();
                for lr in stereo.chunks_exact(2) {
                    in_l.push(lr[0]);
                    in_r.push(lr[1]);
                }
                out_l.clear();
                out_r.clear();
                res_l.process(&in_l, &mut out_l);
                res_r.process(&in_r, &mut out_r);
                queue.extend(out_l.iter().copied().zip(out_r.iter().copied()));
            }
            for frame in data.chunks_mut(channels) {
                let (l, r) = queue.pop_front().unwrap_or((0.0, 0.0));
                match frame.len() {
                    1 => frame[0] = T::from_sample((l + r) * 0.5),
                    _ => {
                        frame[0] = T::from_sample(l);
                        frame[1] = T::from_sample(r);
                        for extra in &mut frame[2..] {
                            *extra = T::EQUILIBRIUM;
                        }
                    }
                }
            }
        },
        |e| log::error!("audio output error: {e}"),
        None,
    )?;
    Ok(stream)
}
