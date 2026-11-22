/// Mixes interleaved frames down to mono.
pub fn to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resamples mono audio with linear interpolation. Downsampling first
/// averages over the source span of each output sample, which is a cheap
/// low-pass filter that is plenty for speech recognition.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        if ratio > 1.0 {
            let start = pos.floor() as usize;
            let end = ((pos + ratio).ceil() as usize)
                .min(input.len())
                .max(start + 1);
            let span = &input[start..end];
            out.push(span.iter().sum::<f32>() / span.len() as f32);
        } else {
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            out.push(a + (b - a) * frac);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_48k_to_16k() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48.0).sin()).collect();
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn upsamples_8k_to_16k() {
        let out = resample(&[0.0, 1.0, 0.0, -1.0], 8_000, 16_000);
        assert_eq!(out.len(), 8);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mixes_stereo() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }
}
