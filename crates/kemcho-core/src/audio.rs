//! Microphone capture. One worker thread owns the cpal stream (streams are
//! not `Send` on every platform); the stream is opened when a recording
//! starts and closed when it stops, so the microphone is only live while
//! you speak.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};

use crate::{SAMPLE_RATE, resample};

/// Recordings are capped at five minutes to bound memory.
const MAX_SECONDS: usize = 300;

pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| {
            devices
                .filter_map(|d| d.description().ok().map(|d| d.name().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

enum Command {
    Start(Option<String>, Sender<Result<()>>),
    Stop(Sender<Result<Vec<f32>>>),
}

#[derive(Clone)]
pub struct Recorder {
    tx: Sender<Command>,
    level: Arc<AtomicU32>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let level = Arc::new(AtomicU32::new(0));
        let worker_level = level.clone();
        thread::Builder::new()
            .name("kemcho-audio".into())
            .spawn(move || worker(rx, worker_level))
            .expect("spawn audio thread");
        Self { tx, level }
    }

    /// Opens the named microphone (or the system default) and starts recording.
    pub fn start(&self, device: Option<String>) -> Result<()> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Command::Start(device, reply))
            .map_err(|_| anyhow!("audio thread stopped"))?;
        rx.recv()?
    }

    /// Stops recording and returns the clip as 16 kHz mono.
    pub fn stop(&self) -> Result<Vec<f32>> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Command::Stop(reply))
            .map_err(|_| anyhow!("audio thread stopped"))?;
        rx.recv()?
    }

    /// The latest input peak in [0, 1].
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

struct Active {
    _stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    channels: usize,
    rate: u32,
}

fn worker(rx: Receiver<Command>, level: Arc<AtomicU32>) {
    let mut active: Option<Active> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::Start(device, reply) => {
                active = None;
                let result = open(device.as_deref(), level.clone()).map(|a| {
                    active = Some(a);
                });
                let _ = reply.send(result);
            }
            Command::Stop(reply) => {
                let result = match active.take() {
                    Some(a) => {
                        let Active {
                            _stream,
                            buffer,
                            channels,
                            rate,
                        } = a;
                        drop(_stream);
                        level.store(0, Ordering::Relaxed);
                        let raw = std::mem::take(&mut *buffer.lock().unwrap());
                        let mono = resample::to_mono(&raw, channels);
                        Ok(resample::resample(&mono, rate, SAMPLE_RATE))
                    }
                    None => Err(anyhow!("not recording")),
                };
                let _ = reply.send(result);
            }
        }
    }
}

fn find_device(host: &cpal::Host, name: Option<&str>) -> Result<cpal::Device> {
    if let Some(name) = name {
        if let Ok(mut devices) = host.input_devices()
            && let Some(d) = devices.find(|d| d.description().is_ok_and(|desc| desc.name() == name))
        {
            return Ok(d);
        }
        tracing::warn!("microphone `{name}` not found; using the default");
    }
    host.default_input_device().context("no microphone found")
}

fn open(name: Option<&str>, level: Arc<AtomicU32>) -> Result<Active> {
    let host = cpal::default_host();
    let device = find_device(&host, name)?;
    let config = device
        .default_input_config()
        .context("microphone has no input configuration")?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate();
    let cap = MAX_SECONDS * rate as usize * channels;
    let buffer = Arc::new(Mutex::new(Vec::with_capacity(
        rate as usize * channels * 10,
    )));
    let err_fn = |err| tracing::error!("microphone stream error: {err}");

    macro_rules! build {
        ($t:ty) => {{
            let buffer = buffer.clone();
            let level = level.clone();
            device.build_input_stream(
                config.clone().into(),
                move |data: &[$t], _: &_| {
                    let mut peak = 0f32;
                    let mut buf = buffer.lock().unwrap();
                    for s in data {
                        let v: f32 = s.to_sample::<f32>();
                        peak = peak.max(v.abs());
                        if buf.len() < cap {
                            buf.push(v);
                        }
                    }
                    level.store(peak.min(1.0).to_bits(), Ordering::Relaxed);
                },
                err_fn,
                None,
            )?
        }};
    }

    let stream = match config.sample_format() {
        SampleFormat::F32 => build!(f32),
        SampleFormat::I16 => build!(i16),
        SampleFormat::U16 => build!(u16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::I8 => build!(i8),
        other => bail!("unsupported microphone sample format {other:?}"),
    };
    stream.play().context("starting the microphone")?;
    Ok(Active {
        _stream: stream,
        buffer,
        channels,
        rate,
    })
}
