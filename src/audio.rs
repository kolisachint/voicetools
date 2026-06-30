//! Audio helpers: the target sample rate, mono downmixing, resampling to
//! 16 kHz, and loading WAV files (for `--wav` and tests).
//!
//! The Parakeet and Whisper models both expect **16 kHz mono f32** in
//! `-1.0..=1.0`. Microphones rarely run at 16 kHz, so we resample whatever
//! the device gives us. A simple linear interpolator is used; it provides
//! mild low-pass behaviour that is adequate for speech recognition and keeps
//! the dependency surface (and binary size) small.

use std::path::Path;

/// Sample rate every backend expects.
pub const TARGET_RATE: u32 = 16_000;

/// Average interleaved multi-channel samples down to mono.
pub fn to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let ch = channels as usize;
    interleaved
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Resample mono `input` from `from_rate` to [`TARGET_RATE`] via linear
/// interpolation. Returns the input unchanged when it's already at the
/// target rate.
pub fn resample_to_16k(input: &[f32], from_rate: u32) -> Vec<f32> {
    resample(input, from_rate, TARGET_RATE)
}

/// Linear-interpolation resampler between arbitrary rates.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.len() < 2 {
        return input.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    let step = from_rate as f64 / to_rate as f64;
    let last = input.len() - 1;
    for i in 0..out_len {
        let src = i as f64 * step;
        let idx = src.floor() as usize;
        if idx >= last {
            out.push(input[last]);
            continue;
        }
        let frac = (src - idx as f64) as f32;
        out.push(input[idx] * (1.0 - frac) + input[idx + 1] * frac);
    }
    out
}

/// Load a WAV file as 16 kHz mono f32. Supports integer and float PCM at any
/// rate/channel count; everything is downmixed and resampled.
pub fn load_wav_16k_mono<P: AsRef<Path>>(path: P) -> anyhow::Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };

    let mono = to_mono(&samples, spec.channels);
    Ok(resample_to_16k(&mono, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_passthrough() {
        let s = vec![0.1, 0.2, 0.3];
        assert_eq!(to_mono(&s, 1), s);
    }

    #[test]
    fn stereo_downmix_averages_channels() {
        // L/R interleaved: (0,1),(2,3) -> 0.5, 2.5
        let s = vec![0.0, 1.0, 2.0, 3.0];
        assert_eq!(to_mono(&s, 2), vec![0.5, 2.5]);
    }

    #[test]
    fn resample_same_rate_is_identity() {
        let s = vec![0.0, 0.5, 1.0];
        assert_eq!(resample(&s, 16_000, 16_000), s);
    }

    #[test]
    fn downsample_halves_length() {
        // 48k -> 16k is a 1/3 ratio.
        let s: Vec<f32> = (0..300).map(|i| i as f32).collect();
        let out = resample_to_16k(&s, 48_000);
        assert_eq!(out.len(), 100);
        // Endpoints are preserved.
        assert!((out[0] - 0.0).abs() < 1e-3);
    }

    #[test]
    fn upsample_preserves_endpoints() {
        let s = vec![0.0, 10.0];
        let out = resample(&s, 8_000, 16_000);
        assert_eq!(out.len(), 4);
        assert!((out[0] - 0.0).abs() < 1e-3);
    }
}
