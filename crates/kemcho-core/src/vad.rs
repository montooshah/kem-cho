//! Energy-based speech detection. Whisper invents text ("Thank you.") for
//! silence, so silent clips never reach it, and quiet edges are trimmed.

const FRAME: usize = 480; // 30 ms at 16 kHz
const PAD_FRAMES: usize = 8; // keep 240 ms around speech

fn rms(frame: &[f32]) -> f32 {
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt()
}

/// Threshold adapts to the clip's noise floor but never drops below `min_rms`.
fn threshold(frames: &[f32], min_rms: f32) -> f32 {
    let mut sorted = frames.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted.get(sorted.len() / 10).copied().unwrap_or(0.0);
    (floor * 3.0).max(min_rms)
}

/// Returns the speech span of a 16 kHz clip, or `None` when it holds no speech.
pub fn trim(samples: &[f32], min_rms: f32) -> Option<&[f32]> {
    let energies: Vec<f32> = samples.chunks(FRAME).map(rms).collect();
    let t = threshold(&energies, min_rms);
    let voiced: Vec<usize> = energies
        .iter()
        .enumerate()
        .filter(|(_, e)| **e > t)
        .map(|(i, _)| i)
        .collect();
    // At least ~90 ms of voiced frames counts as speech.
    if voiced.len() < 3 {
        return None;
    }
    let first = voiced[0].saturating_sub(PAD_FRAMES);
    let last = (voiced[voiced.len() - 1] + PAD_FRAMES + 1).min(energies.len());
    let start = first * FRAME;
    let end = (last * FRAME).min(samples.len());
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
}
