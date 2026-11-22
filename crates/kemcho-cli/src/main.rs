use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use kemcho_core::settings::Settings;
use kemcho_core::transcribe::Options;
use kemcho_core::{Transcriber, agent, audio, models, postprocess, tts, vad, wav};

#[derive(Parser)]
#[command(
    name = "kemcho",
    version,
    about = "Kem Cho: local voice dictation and voice agent"
)]
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
    /// Speak text with the reply voice.
    Say {
        text: String,
        /// Write a 16 kHz WAV instead of playing it.
        #[arg(long)]
        save: Option<PathBuf>,
    },
    /// Ask the local agent a question in text; speaks the reply.
    Ask {
        question: String,
        #[arg(long)]
        quiet: bool,
    },
    /// Voice agent: record a question, answer it aloud.
    Chat {
        #[arg(long, default_value_t = 5)]
        seconds: u64,
        /// Use this WAV as the question instead of the microphone.
        #[arg(long)]
        file: Option<PathBuf>,
    },
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
        Command::Say { text, save } => {
            ensure_voice(&settings)?;
            match save {
                Some(path) => {
                    wav::write_16k_mono(&path, &tts::synthesize(&settings.agent_voice, &text)?)?
                }
                None => {
                    let speech = tts::Speech::start(&settings.agent_voice, Arc::default())?;
                    speech.say(&text);
                    speech.finish()?;
                }
            }
        }
        Command::Ask { question, quiet } => ask(&settings, &question, !quiet)?,
        Command::Chat { seconds, file } => {
            let samples = match file {
                Some(f) => wav::read_16k_mono(&f)?,
                None => {
                    let recorder = audio::Recorder::new();
                    recorder.start(settings.microphone.clone())?;
                    eprintln!("Listening for {seconds}s… ask something.");
                    std::thread::sleep(Duration::from_secs(seconds));
                    recorder.stop()?
                }
            };
            let question = run(&settings, None, None, &samples, false)?;
            println!("You: {question}");
            ask(&settings, &question, true)?;
        }
    }
    Ok(())
}

fn ensure_voice(settings: &Settings) -> Result<()> {
    let voice = tts::find_voice(&settings.agent_voice)?;
    if !voice.is_installed() {
        let mut last = 0;
        voice.download(|done, total| {
            let pct = done * 100 / total.max(1);
            if pct != last {
                last = pct;
                eprint!("\rDownloading voice {pct:>3}%");
                let _ = std::io::stderr().flush();
            }
        })?;
        eprintln!();
    }
    Ok(())
}

fn ask(settings: &Settings, question: &str, speak: bool) -> Result<()> {
    let ollama = agent::Ollama::new(&settings.ollama_url);
    let messages = agent::Conversation::default().prompt(&settings.agent_personality, question);
    let cancel = Arc::new(AtomicBool::new(false));
    let speech = if speak {
        ensure_voice(settings)?;
        Some(tts::Speech::start(&settings.agent_voice, cancel.clone())?)
    } else {
        None
    };
    let mut sentences = agent::Sentences::default();
    let started = Instant::now();
    let mut first = None;
    print!("Kem Cho: ");
    ollama.chat(&settings.agent_model, &messages, &cancel, |token| {
        first.get_or_insert_with(|| started.elapsed());
        print!("{token}");
        let _ = std::io::stdout().flush();
        for s in sentences.push(token) {
            if let Some(sp) = &speech {
                sp.say(&s);
            }
        }
    })?;
    println!();
    if let (Some(sp), Some(rest)) = (&speech, sentences.finish()) {
        sp.say(&rest);
    }
    if let Some(first) = first {
        eprintln!("(first words after {} ms)", first.as_millis());
    }
    if let Some(sp) = speech {
        sp.finish()?;
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
