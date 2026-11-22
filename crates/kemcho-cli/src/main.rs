use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use kemcho_core::settings::Settings;
use kemcho_core::transcribe::Options;
use kemcho_core::{Transcriber, audio, models, postprocess, vad, wav};

#[derive(Parser)]
#[command(name = "kemcho", version, about = "Kem Cho: local voice dictation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List speech models and whether they are installed.
    Models,
    /// Download and verify a speech model.
    Download { model: Option<String> },
    /// Transcribe a WAV file.
    Transcribe {
        file: PathBuf,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        language: Option<String>,
        /// Print Whisper's text without clean-up steps.
        #[arg(long)]
        raw: bool,
    },
    /// Record from the microphone for a few seconds and transcribe it.
    Record {
        #[arg(long, default_value_t = 5)]
        seconds: u64,
        #[arg(long)]
        model: Option<String>,
        /// Also save the recording as a WAV file.
        #[arg(long)]
        save: Option<PathBuf>,
    },
    /// List microphones.
    Devices,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    whisper_rs::install_logging_hooks();
    let cli = Cli::parse();
    let settings = Settings::load();
    match cli.command {
        Command::Models => {
            for m in models::CATALOG {
                let mark = if m.is_installed() { "installed" } else { "" };
                println!(
                    "{:<22} {:>5} MB  {:<10} {}",
                    m.id,
                    m.size_bytes / 1_000_000,
                    mark,
                    m.description
                );
            }
        }
        Command::Download { model } => {
            let m = models::find(model.as_deref().unwrap_or(&settings.model))?;
            download(m)?;
        }
        Command::Transcribe {
            file,
            model,
            language,
            raw,
        } => {
            let samples = wav::read_16k_mono(&file)?;
            let text = run(&settings, model, language, &samples, raw)?;
            println!("{text}");
        }
        Command::Record {
            seconds,
            model,
            save,
        } => {
            let recorder = audio::Recorder::new();
            recorder.start(settings.microphone.clone())?;
            eprintln!("Recording for {seconds}s… speak now.");
            std::thread::sleep(Duration::from_secs(seconds));
            let samples = recorder.stop()?;
            if let Some(path) = save {
                wav::write_16k_mono(&path, &samples)?;
            }
            let text = run(&settings, model, None, &samples, false)?;
            println!("{text}");
        }
        Command::Devices => {
            for d in audio::input_devices() {
                println!("{d}");
            }
        }
    }
    Ok(())
}

fn download(m: &'static models::Model) -> Result<()> {
    let mut last = 0;
    m.download(|done, total| {
        let pct = done * 100 / total.max(1);
        if pct != last {
            last = pct;
            eprint!("\rDownloading {} {pct:>3}%", m.id);
            let _ = std::io::stderr().flush();
        }
    })?;
    eprintln!("\r{} is installed.            ", m.id);
    Ok(())
}

fn run(
    settings: &Settings,
    model: Option<String>,
    language: Option<String>,
    samples: &[f32],
    raw: bool,
) -> Result<String> {
    let m = models::find(model.as_deref().unwrap_or(&settings.model))?;
    if !m.is_installed() {
        download(m)?;
    }
    let Some(speech) = vad::trim(samples, vad::DEFAULT_MIN_RMS) else {
        bail!("no speech detected");
    };
    let transcriber = Transcriber::load(m)?;
    let language = language.unwrap_or_else(|| settings.language.clone());
    let out = transcriber.transcribe(
        speech,
        &Options {
            language: &language,
            vocabulary: &settings.vocabulary,
            threads: None,
        },
    )?;
    Ok(if raw {
        out.text
    } else {
        postprocess::process(&out.text, &settings.steps())
    })
}
