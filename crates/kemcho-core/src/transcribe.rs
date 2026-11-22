use std::time::Instant;

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::SAMPLE_RATE;
use crate::models::Model;

pub struct Options<'a> {
    /// ISO 639-1 code such as `en` or `gu`, or `auto` to detect it.
    pub language: &'a str,
    /// Names and terms Whisper should prefer.
    pub vocabulary: &'a [String],
    pub threads: Option<i32>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            language: "en",
            vocabulary: &[],
            threads: None,
        }
    }
}

pub struct Transcript {
    pub text: String,
    pub elapsed_ms: u64,
}

/// A loaded Whisper model. Loading takes a moment, so keep one around.
pub struct Transcriber {
    ctx: WhisperContext,
    pub model: &'static Model,
}

impl Transcriber {
    pub fn load(model: &'static Model) -> Result<Self> {
        let path = model.path()?;
        let path_str = path.to_str().context("model path is not UTF-8")?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .with_context(|| format!("loading {}", path.display()))?;
        Ok(Self { ctx, model })
    }

    /// Transcribes 16 kHz mono audio.
    pub fn transcribe(&self, samples: &[f32], opts: &Options) -> Result<Transcript> {
        let started = Instant::now();
        // whisper.cpp refuses clips shorter than one second; pad with silence.
        let min = SAMPLE_RATE as usize + SAMPLE_RATE as usize / 10;
        let mut audio = samples.to_vec();
        if audio.len() < min {
            audio.resize(min, 0.0);
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let language = if self.model.english_only {
            "en"
        } else {
            opts.language
        };
        params.set_language(Some(language));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(false);
        params.set_suppress_blank(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let threads = opts.threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get().min(8) as i32)
                .unwrap_or(4)
        });
        params.set_n_threads(threads);
        let prompt = vocabulary_prompt(opts.vocabulary);
        if !prompt.is_empty() {
            params.set_initial_prompt(&prompt);
        }

        let mut state = self.ctx.create_state().context("creating Whisper state")?;
        state.full(params, &audio).context("running Whisper")?;
        let mut text = String::new();
        for segment in state.as_iter() {
            let piece = segment.to_str_lossy()?.trim().to_string();
            if piece.is_empty() || is_non_speech(&piece) {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&piece);
        }
        Ok(Transcript {
            text,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn vocabulary_prompt(words: &[String]) -> String {
    let words: Vec<&str> = words
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty() && !w.contains('\0'))
        .collect();
    if words.is_empty() {
        String::new()
    } else {
        format!("Vocabulary: {}.", words.join(", "))
    }
}

/// Whisper marks noise as `[BLANK_AUDIO]`, `(music)`, `*coughs*` and the like.
pub fn is_non_speech(segment: &str) -> bool {
    let s = segment.trim();
    let wrapped = |open: char, close: char| s.starts_with(open) && s.ends_with(close);
    wrapped('[', ']') || wrapped('(', ')') || wrapped('*', '*') || s == "♪" || s == "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_noise_markers() {
        assert!(is_non_speech("[BLANK_AUDIO]"));
        assert!(is_non_speech(" (upbeat music) "));
        assert!(!is_non_speech("Hello (world) there"));
    }

    #[test]
    fn builds_prompt() {
        assert_eq!(vocabulary_prompt(&[]), "");
        let words = vec!["Kem Cho".to_string(), " ".to_string(), "Tauri".to_string()];
        assert_eq!(vocabulary_prompt(&words), "Vocabulary: Kem Cho, Tauri.");
    }
}
