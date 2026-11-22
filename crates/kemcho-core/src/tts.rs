//! Spoken replies. Piper (a small neural voice) runs as a child process on
//! Linux and Windows; macOS uses its built-in voices through `say`. Both run
//! entirely on this computer.

use std::collections::VecDeque;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{paths, resample};

const VOICES_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0";
const PIPER_URL: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2";

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Voice {
    pub id: &'static str,
    pub label: &'static str,
    /// Path inside the piper-voices repository, without the extension.
    #[serde(skip)]
    pub repo_path: &'static str,
    pub size_bytes: u64,
    #[serde(skip)]
    pub onnx_sha256: &'static str,
    #[serde(skip)]
    pub json_sha256: &'static str,
}

pub const VOICES: &[Voice] = &[
    Voice {
        id: "en_US-lessac-medium",
        label: "Lessac (US English, female)",
        repo_path: "en/en_US/lessac/medium/en_US-lessac-medium",
        size_bytes: 63_201_294,
        onnx_sha256: "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f",
        json_sha256: "efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0",
    },
    Voice {
        id: "en_US-ryan-medium",
        label: "Ryan (US English, male)",
        repo_path: "en/en_US/ryan/medium/en_US-ryan-medium",
        size_bytes: 63_201_294,
        onnx_sha256: "abf4c274862564ed647ba0d2c47f8ee7c9b717d27bdad9219100eb310db4047a",
        json_sha256: "44034c056cb15681b2ad494307c7f3f2e4499d1253c700c711fa0a4607ffe78d",
    },
    Voice {
        id: "en_US-amy-medium",
        label: "Amy (US English, female)",
        repo_path: "en/en_US/amy/medium/en_US-amy-medium",
        size_bytes: 63_201_294,
        onnx_sha256: "b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18",
        json_sha256: "95a23eb4d42909d38df73bb9ac7f45f597dbfcde2d1bf9526fdeaf5466977d77",
    },
    Voice {
        id: "en_GB-alba-medium",
        label: "Alba (British English, female)",
        repo_path: "en/en_GB/alba/medium/en_GB-alba-medium",
        size_bytes: 63_201_294,
        onnx_sha256: "401369c4a81d09fdd86c32c5c864440811dbdcc66466cde2d64f7133a66ad03b",
        json_sha256: "aa965a2f02ecced632c2694e1fc72bbff6d65f265fab567ca945918c73dd89f4",
    },
];

pub const DEFAULT_VOICE: &str = "en_US-lessac-medium";
/// Every bundled voice speaks at this rate.
const PIPER_RATE: u32 = 22_050;

struct PiperBuild {
    asset: &'static str,
    size_bytes: u64,
    sha256: &'static str,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PIPER: Option<PiperBuild> = Some(PiperBuild {
    asset: "piper_linux_x86_64.tar.gz",
    size_bytes: 26_460_462,
    sha256: "a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992",
});
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const PIPER: Option<PiperBuild> = Some(PiperBuild {
    asset: "piper_linux_aarch64.tar.gz",
    size_bytes: 26_004_717,
    sha256: "fea0fd2d87c54dbc7078d0f878289f404bd4d6eea6e7444a77835d1537ab88eb",
});
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const PIPER: Option<PiperBuild> = Some(PiperBuild {
    asset: "piper_windows_amd64.zip",
    size_bytes: 22_477_236,
    sha256: "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea",
});
#[cfg(not(any(
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(target_os = "windows", target_arch = "x86_64"),
)))]
const PIPER: Option<PiperBuild> = None;

/// Whether this platform speaks with Piper voices (otherwise the system voice).
pub fn uses_piper() -> bool {
    PIPER.is_some()
}

pub fn find_voice(id: &str) -> Result<&'static Voice> {
    VOICES
        .iter()
        .find(|v| v.id == id)
        .ok_or_else(|| anyhow!("unknown voice `{id}`"))
}

fn piper_dir() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("piper"))
}

fn piper_exe() -> Result<PathBuf> {
    let name = if cfg!(windows) { "piper.exe" } else { "piper" };
    Ok(piper_dir()?.join(name))
}

fn voices_dir() -> Result<PathBuf> {
    let dir = paths::data_dir()?.join("voices");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl Voice {
    pub fn path(&self) -> Result<PathBuf> {
        Ok(voices_dir()?.join(format!("{}.onnx", self.id)))
    }

    /// The voice and, where needed, the Piper engine are on disk.
    pub fn is_installed(&self) -> bool {
        if !uses_piper() {
            return true;
        }
        let config = self.path().map(|p| p.with_extension("onnx.json"));
        self.path().is_ok_and(|p| p.exists())
            && config.is_ok_and(|p| p.exists())
            && piper_exe().is_ok_and(|p| p.exists())
    }

    /// Downloads Piper (once) and this voice, reporting combined progress.
    pub fn download(&self, mut progress: impl FnMut(u64, u64)) -> Result<()> {
        let Some(build) = &PIPER else { return Ok(()) };
        let engine = if piper_exe()?.exists() {
            0
        } else {
            build.size_bytes
        };
        let total = engine + self.size_bytes;
        if engine > 0 {
            let archive = paths::data_dir()?.join(build.asset);
            fetch(
                &format!("{PIPER_URL}/{}", build.asset),
                &archive,
                build.sha256,
                |d| progress(d, total),
            )?;
            let result = extract(&archive, &paths::data_dir()?);
            let _ = std::fs::remove_file(&archive);
            result.context("unpacking Piper")?;
        }
        let onnx = self.path()?;
        if !onnx.exists() {
            fetch(
                &format!("{VOICES_URL}/{}.onnx", self.repo_path),
                &onnx,
                self.onnx_sha256,
                |d| progress(engine + d, total),
            )?;
        }
        fetch(
            &format!("{VOICES_URL}/{}.onnx.json", self.repo_path),
            &onnx.with_extension("onnx.json"),
            self.json_sha256,
            |_| {},
        )?;
        progress(total, total);
        Ok(())
    }
}

/// Downloads `url` to `dest`, keeping it only if its SHA-256 matches.
fn fetch(url: &str, dest: &Path, sha256: &str, mut progress: impl FnMut(u64)) -> Result<()> {
    let partial = dest.with_extension("partial");
    let response = ureq::get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let mut reader = response.into_body().into_reader();
    let mut file = BufWriter::new(std::fs::File::create(&partial)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).context("download interrupted")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done);
    }
    file.flush()?;
    drop(file);
    let actual = crate::models::hex(&hasher.finalize());
    if actual != sha256 {
        let _ = std::fs::remove_file(&partial);
        bail!("{url} failed verification (expected {sha256}, got {actual})");
    }
    std::fs::rename(&partial, dest)?;
    Ok(())
}

fn extract(archive: &Path, into: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    if archive.extension().is_some_and(|e| e == "zip") {
        zip::ZipArchive::new(file)?.extract(into)?;
    } else {
        tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(into)?;
    }
    Ok(())
}

/// One spoken reply: feed it sentences as they arrive, then wait for it to
/// finish. Setting `cancel` stops the voice at once.
pub struct Speech {
    tx: Option<Sender<String>>,
    worker: Option<JoinHandle<Result<()>>>,
}

impl Speech {
    pub fn start(voice: &str, cancel: Arc<AtomicBool>) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<String>();
        let worker = if uses_piper() {
            let voice = find_voice(voice)?;
            if !voice.is_installed() {
                bail!("Download the {} voice first.", voice.label);
            }
            let model = voice.path()?;
            thread::Builder::new()
                .name("kemcho-speech".into())
                .spawn(move || speak_piper(&model, rx, cancel))?
        } else {
            thread::Builder::new()
                .name("kemcho-speech".into())
                .spawn(move || speak_system(rx, cancel))?
        };
        Ok(Self {
            tx: Some(tx),
            worker: Some(worker),
        })
    }

    pub fn say(&self, sentence: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(sentence.replace(['\n', '\r'], " "));
        }
    }

    /// Blocks until everything said so far has been spoken or cancelled.
    pub fn finish(mut self) -> Result<()> {
        self.tx = None;
        match self.worker.take() {
            Some(w) => w.join().map_err(|_| anyhow!("speech thread panicked"))?,
            None => Ok(()),
        }
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

struct Piper {
    child: Child,
    stdin: ChildStdin,
}

fn spawn_piper(model: &Path) -> Result<Piper> {
    let exe = piper_exe()?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--model")
        .arg(model)
        .arg("--espeak_data")
        .arg(piper_dir()?.join("espeak-ng-data"))
        .args(["--output_raw", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", exe.display()))?;
    let stdin = child.stdin.take().context("piper stdin")?;
    Ok(Piper { child, stdin })
}

fn speak_piper(model: &Path, rx: Receiver<String>, cancel: Arc<AtomicBool>) -> Result<()> {
    let Piper { mut child, stdin } = spawn_piper(model)?;
    let mut stdout = child.stdout.take().context("piper stdout")?;
    let player = Player::open(cancel.clone())?;
    let queue = player.queue.clone();
    let rate = player.rate;

    let writer = thread::spawn(move || {
        let mut stdin = stdin;
        for sentence in rx {
            if writeln!(stdin, "{sentence}")
                .and_then(|_| stdin.flush())
                .is_err()
            {
                break;
            }
        }
    });

    let mut bytes = vec![0u8; 4410 * 2];
    let mut carry: Option<u8> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
        let n = match stdout.read(&mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut data: Vec<u8> = carry.take().into_iter().collect();
        data.extend_from_slice(&bytes[..n]);
        if data.len() % 2 == 1 {
            carry = data.pop();
        }
        let samples: Vec<f32> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b) as f32 / 32768.0)
            .collect();
        let resampled = resample::resample(&samples, PIPER_RATE, rate);
        queue.lock().unwrap().extend(resampled);
    }
    let _ = writer.join();
    let _ = child.wait();
    player.drain();
    Ok(())
}

fn speak_system(rx: Receiver<String>, cancel: Arc<AtomicBool>) -> Result<()> {
    for sentence in rx {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut child = system_voice(&sentence)?
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting the system voice")?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                break;
            }
            if child.try_wait()?.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
    Ok(())
}

fn system_voice(text: &str) -> Result<Command> {
    if cfg!(target_os = "macos") {
        let mut cmd = Command::new("say");
        cmd.arg("--").arg(text);
        Ok(cmd)
    } else if cfg!(windows) {
        let mut cmd = Command::new("powershell");
        let escaped = text.replace('\'', "''");
        cmd.args([
            "-NoProfile",
            "-Command",
            &format!(
                "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak('{escaped}')"
            ),
        ]);
        Ok(cmd)
    } else {
        let mut cmd = Command::new("espeak-ng");
        cmd.arg("--").arg(text);
        Ok(cmd)
    }
}

/// Plays queued mono samples on the default output device. Its own thread
/// owns the cpal stream.
struct Player {
    queue: Arc<Mutex<VecDeque<f32>>>,
    rate: u32,
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
}

impl Player {
    fn open(cancel: Arc<AtomicBool>) -> Result<Self> {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop, stop_rx) = mpsc::channel::<()>();
        let q = queue.clone();
        let c = cancel.clone();
        let thread = thread::Builder::new()
            .name("kemcho-playback".into())
            .spawn(move || match open_output(q, c) {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            })?;
        let rate = ready_rx
            .recv()
            .map_err(|_| anyhow!("audio output thread stopped"))??;
        Ok(Self {
            queue,
            rate,
            stop,
            thread: Some(thread),
            cancel,
        })
    }

    /// Waits until the queue has played out (or speech was cancelled).
    fn drain(mut self) {
        while !self.cancel.load(Ordering::Relaxed) && !self.queue.lock().unwrap().is_empty() {
            thread::sleep(Duration::from_millis(20));
        }
        // Let the device play its last buffer.
        thread::sleep(Duration::from_millis(150));
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn open_output(
    queue: Arc<Mutex<VecDeque<f32>>>,
    cancel: Arc<AtomicBool>,
) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("no speakers or headphones found")?;
    let config = device
        .default_output_config()
        .context("audio output has no configuration")?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate();

    fn build<T: SizedSample + FromSample<f32>>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        queue: Arc<Mutex<VecDeque<f32>>>,
        cancel: Arc<AtomicBool>,
    ) -> Result<cpal::Stream> {
        Ok(device.build_output_stream(
            *config,
            move |out: &mut [T], _: &_| {
                let mut q = queue.lock().unwrap();
                if cancel.load(Ordering::Relaxed) {
                    q.clear();
                }
                for frame in out.chunks_mut(channels) {
                    let v = q.pop_front().unwrap_or(0.0);
                    frame.fill(T::from_sample(v));
                }
            },
            |err| tracing::error!("audio output error: {err}"),
            None,
        )?)
    }

    let stream_config: cpal::StreamConfig = config.into();
    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &stream_config, channels, queue, cancel)?,
        SampleFormat::I16 => build::<i16>(&device, &stream_config, channels, queue, cancel)?,
        SampleFormat::U16 => build::<u16>(&device, &stream_config, channels, queue, cancel)?,
        SampleFormat::I32 => build::<i32>(&device, &stream_config, channels, queue, cancel)?,
        other => bail!("unsupported audio output format {other:?}"),
    };
    stream.play().context("starting audio output")?;
    Ok((stream, rate))
}

/// Renders `text` with the voice to 16 kHz mono, for tests and previews.
pub fn synthesize(voice: &str, text: &str) -> Result<Vec<f32>> {
    if !uses_piper() {
        bail!("offline synthesis needs Piper, which this platform doesn't use");
    }
    let voice = find_voice(voice)?;
    let Piper {
        mut child,
        mut stdin,
    } = spawn_piper(&voice.path()?)?;
    writeln!(stdin, "{}", text.replace('\n', " "))?;
    drop(stdin);
    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .context("piper stdout")?
        .read_to_end(&mut raw)?;
    child.wait()?;
    let samples: Vec<f32> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| i16::from_le_bytes(b) as f32 / 32768.0)
        .collect();
    Ok(resample::resample(&samples, PIPER_RATE, crate::SAMPLE_RATE))
}
