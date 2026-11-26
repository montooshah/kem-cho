//! Kem Cho's engine: microphone capture, local Whisper transcription,
//! transcript clean-up, settings and history. Nothing here talks to the
//! network except the explicit model download.

pub mod agent;
pub mod audio;
pub mod history;
pub mod intent;
pub mod languages;
pub mod models;
pub mod paths;
pub mod postprocess;
pub mod resample;
pub mod settings;
pub mod transcribe;
pub mod tts;
pub mod vad;
pub mod wav;

pub use transcribe::Transcriber;

/// Whisper's input rate.
pub const SAMPLE_RATE: u32 = 16_000;
