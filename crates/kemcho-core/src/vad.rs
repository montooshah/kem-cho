//! Energy-based speech detection. Whisper invents text ("Thank you.") for
//! silence, so silent clips never reach it, and quiet edges are trimmed.

const FRAME: usize = 480; // 30 ms at 16 kHz
const PAD_FRAMES: usize = 8; // keep 240 ms around speech

fn rms(frame: &[f32]) -> f32 {
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt()
}

/// Frames this far above the clip's noise floor count as voiced.
fn floor(frames: &[f32]) -> f32 {
    let mut sorted = frames.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    sorted.get(sorted.len() / 10).copied().unwrap_or(0.0) * 3.0
}

/// Returns the speech span of a 16 kHz clip, or `None` when it holds no speech.
///
/// A clip counts as speech once a few frames pass `min_rms`, but its edges are
/// found with a lower bar so quiet words at the end of a sentence are kept.
pub fn trim(samples: &[f32], min_rms: f32) -> Option<&[f32]> {
    let energies: Vec<f32> = samples.chunks(FRAME).map(rms).collect();
    let floor = floor(&energies);
    // At least ~90 ms of loud frames counts as speech.
    if energies.iter().filter(|e| **e > floor.max(min_rms)).count() < 3 {
        return None;
    }
    let edge = floor.max(min_rms / 4.0);
    let first = energies.iter().position(|e| *e > edge)?;
    let last = energies.iter().rposition(|e| *e > edge)?;
    let start = first.saturating_sub(PAD_FRAMES) * FRAME;
    let end = ((last + PAD_FRAMES + 1) * FRAME).min(samples.len());
    Some(&samples[start..end])
}

pub const DEFAULT_MIN_RMS: f32 = 0.008;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_no_speech() {
        assert!(trim(&vec![0.0; 16_000], DEFAULT_MIN_RMS).is_none());
        let hiss: Vec<f32> = (0..16_000)
            .map(|i| ((i * 7919) % 13) as f32 * 1e-4)
            .collect();
        assert!(trim(&hiss, DEFAULT_MIN_RMS).is_none());
    }

    #[test]
    fn trims_to_tone() {
        let mut clip = vec![0.0f32; 32_000];
        for (i, s) in clip[16_000..24_000].iter_mut().enumerate() {
            *s = (i as f32 * 0.1).sin() * 0.3;
        }
        let speech = trim(&clip, DEFAULT_MIN_RMS).unwrap();
        assert!(speech.len() >= 8_000 && speech.len() < 16_000);
    }

    #[test]
    fn keeps_quiet_ending() {
        let tone = |amp: f32| (0..8_000).map(move |i| (i as f32 * 0.1).sin() * amp);
        let clip: Vec<f32> = std::iter::repeat_n(0.0, 8_000)
            .chain(tone(0.3))
            .chain(tone(0.006))
            .chain(std::iter::repeat_n(0.0, 8_000))
            .collect();
        let speech = trim(&clip, DEFAULT_MIN_RMS).unwrap();
        assert!(speech.len() >= 16_000, "{}", speech.len());
    }
}
