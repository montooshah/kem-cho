use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::models::Model;
use crate::{SAMPLE_RATE, languages};

pub struct Options<'a> {
    /// A dictation language such as `hi-en`, or a Whisper code such as `gu` or `auto`.
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
        Self::load_file(model, &model.path()?)
    }

    /// Loads `model`'s settings with weights from another file, for evaluation.
    pub fn load_file(model: &'static Model, path: &Path) -> Result<Self> {
        let path_str = path.to_str().context("model path is not UTF-8")?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .with_context(|| format!("loading {}", path.display()))?;
        Ok(Self { ctx, model })
    }

    /// Transcribes 16 kHz mono audio.
    pub fn transcribe(&self, samples: &[f32], opts: &Options) -> Result<Transcript> {
        let started = Instant::now();
        let pieces = if self.model.no_timestamps {
            split_at_pauses(samples, CHUNK_SECONDS * SAMPLE_RATE as usize)
        } else {
            vec![samples]
        };
        let mut text = String::new();
        for piece in pieces {
            let piece = self.decode(piece, opts)?;
            if !piece.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&piece);
            }
        }
        Ok(Transcript {
            text,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    fn decode(&self, samples: &[f32], opts: &Options) -> Result<String> {
        // whisper.cpp refuses clips shorter than one second; pad with silence.
        let min = SAMPLE_RATE as usize + SAMPLE_RATE as usize / 10;
        let mut audio = samples.to_vec();
        if audio.len() < min {
            audio.resize(min, 0.0);
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let wanted = languages::find(opts.language).map_or(opts.language, |l| l.whisper);
        let language = if self.model.supports(wanted) {
            wanted
        } else {
            self.model.languages[0]
        };
        params.set_language(Some(language));
        params.set_no_timestamps(self.model.no_timestamps);
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

        // A model stuck repeating itself can decode for minutes; give up and keep what it has.
        let seconds = audio.len() as f32 / SAMPLE_RATE as f32;
        let budget = Duration::from_secs(20) + Duration::from_secs_f32(3.0 * seconds);
        let deadline = Instant::now() + budget;
        // SAFETY: `deadline` outlives `state.full`, the only caller of the callback.
        unsafe {
            params.set_abort_callback(Some(past_deadline));
            params.set_abort_callback_user_data(&deadline as *const Instant as *mut c_void);
        }

        let mut state = self.ctx.create_state().context("creating Whisper state")?;
        if let Err(err) = state.full(params, &audio) {
            if Instant::now() <= deadline {
                return Err(err).context("running Whisper");
            }
            tracing::warn!("Whisper took longer than {budget:?}; keeping its partial text");
        }
        let mut text = String::new();
        for segment in state.as_iter() {
            let piece = segment.to_str_lossy()?;
            // The last token of a cut-off decode can end mid-character.
            let piece = piece.trim().trim_end_matches('\u{FFFD}').trim();
            if piece.is_empty() || is_non_speech(piece) {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(piece);
        }
        Ok(text)
    }
}

unsafe extern "C" fn past_deadline(deadline: *mut c_void) -> bool {
    // SAFETY: `decode` passes a pointer to an `Instant` that outlives the decode.
    Instant::now() > unsafe { *(deadline as *const Instant) }
}

/// Longest piece a model without timestamps decodes at once: it stops after one
/// window's worth of tokens, which long Indic sentences can exceed.
const CHUNK_SECONDS: usize = 10;

/// Splits audio longer than `max` samples at the quietest 100 ms in the
/// second half of each `max`-long stretch.
fn split_at_pauses(samples: &[f32], max: usize) -> Vec<&[f32]> {
    const FRAME: usize = SAMPLE_RATE as usize / 10;
    let mut pieces = Vec::new();
    let mut rest = samples;
    while rest.len() > max {
        let cut = (max / 2..max - FRAME)
            .step_by(FRAME / 2)
            .min_by(|&a, &b| energy(&rest[a..a + FRAME]).total_cmp(&energy(&rest[b..b + FRAME])))
            .map_or(max, |start| start + FRAME / 2);
        pieces.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    pieces.push(rest);
    pieces
}

fn energy(frame: &[f32]) -> f32 {
    frame.iter().map(|s| s * s).sum()
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

    #[test]
    fn splits_long_audio_at_the_quietest_point() {
        let rate = SAMPLE_RATE as usize;
        let mut audio = vec![0.5f32; 25 * rate];
        audio[7 * rate..7 * rate + rate / 5].fill(0.0);
        let pieces = split_at_pauses(&audio, 10 * rate);
        assert_eq!(pieces.iter().map(|p| p.len()).sum::<usize>(), audio.len());
        assert!(pieces.iter().all(|p| p.len() <= 10 * rate));
        assert!((7 * rate..7 * rate + rate / 5).contains(&pieces[0].len()));
        assert_eq!(split_at_pauses(&audio[..rate], 10 * rate).len(), 1);
    }
}
