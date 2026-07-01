//! Energy-based voice activity detection with auto-stop on trailing silence.
//!
//! The detector is intentionally simple: it tracks short-term RMS energy and,
//! once it has heard speech, starts a silence timer. When the buffer stays
//! below the threshold for `silence_duration_ms`, [`VadEvent::SilenceTimeout`]
//! fires and the caller flushes the captured audio to the transcriber.
//!
//! Silence *before* the user starts talking never times out, so a slow start
//! won't cut the recording short.

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    /// The current chunk contains speech (energy above threshold).
    Speech,
    /// The current chunk is silent, but we're still waiting (either the user
    /// hasn't spoken yet, or the trailing-silence timer hasn't elapsed).
    Silence,
    /// The first silent chunk after speech — the trailing-silence timer just
    /// started. Fired once per speech run, useful for UI phase markers.
    SilenceStart,
    /// Trailing silence has lasted long enough — stop recording.
    SilenceTimeout,
}

pub struct Vad {
    /// RMS amplitude above which a chunk counts as speech (0.0..1.0).
    silence_threshold: f32,
    /// How long trailing silence must last before auto-stop, in milliseconds.
    silence_duration_ms: u64,
    /// When the current run of silence began, if any.
    silent_since: Option<Instant>,
    /// Whether we've heard speech at least once.
    has_heard_speech: bool,
}

impl Vad {
    /// Create a VAD with the given trailing-silence timeout and a sensible
    /// default energy threshold.
    pub fn new(silence_duration_ms: u64) -> Self {
        Self::with_threshold(silence_duration_ms, 0.01)
    }

    pub fn with_threshold(silence_duration_ms: u64, silence_threshold: f32) -> Self {
        Self {
            silence_threshold,
            silence_duration_ms,
            silent_since: None,
            has_heard_speech: false,
        }
    }

    /// Whether any speech has been observed so far.
    pub fn heard_speech(&self) -> bool {
        self.has_heard_speech
    }

    /// Feed a chunk of mono f32 samples and get the resulting event.
    pub fn push(&mut self, samples: &[f32]) -> VadEvent {
        if samples.is_empty() {
            return VadEvent::Silence;
        }
        let rms = rms(samples);

        if rms > self.silence_threshold {
            self.has_heard_speech = true;
            self.silent_since = None;
            VadEvent::Speech
        } else if !self.has_heard_speech {
            // Don't start the timeout clock until the user actually speaks.
            VadEvent::Silence
        } else if let Some(started) = self.silent_since {
            if started.elapsed().as_millis() >= self.silence_duration_ms as u128 {
                VadEvent::SilenceTimeout
            } else {
                VadEvent::Silence
            }
        } else {
            self.silent_since = Some(Instant::now());
            VadEvent::SilenceStart
        }
    }
}

/// Root-mean-square amplitude of a sample buffer.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    fn loud(n: usize) -> Vec<f32> {
        vec![0.5; n]
    }
    fn quiet(n: usize) -> Vec<f32> {
        vec![0.0; n]
    }

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&quiet(100)), 0.0);
    }

    #[test]
    fn rms_of_constant_amplitude() {
        // RMS of a constant 0.5 signal is 0.5.
        assert!((rms(&loud(100)) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn no_timeout_before_speech() {
        let mut vad = Vad::with_threshold(10, 0.01);
        // Silence forever before any speech must never time out.
        for _ in 0..5 {
            assert_eq!(vad.push(&quiet(160)), VadEvent::Silence);
            sleep(Duration::from_millis(5));
        }
        assert!(!vad.heard_speech());
    }

    #[test]
    fn speech_then_silence_times_out() {
        let mut vad = Vad::with_threshold(30, 0.01);
        assert_eq!(vad.push(&loud(160)), VadEvent::Speech);
        assert!(vad.heard_speech());

        // First silent chunk starts the clock but shouldn't time out yet.
        assert_eq!(vad.push(&quiet(160)), VadEvent::SilenceStart);
        sleep(Duration::from_millis(50));
        assert_eq!(vad.push(&quiet(160)), VadEvent::SilenceTimeout);
    }

    #[test]
    fn speech_resets_silence_timer() {
        let mut vad = Vad::with_threshold(50, 0.01);
        vad.push(&loud(160));
        assert_eq!(vad.push(&quiet(160)), VadEvent::SilenceStart);
        sleep(Duration::from_millis(30));
        // Speaking again clears the timer.
        assert_eq!(vad.push(&loud(160)), VadEvent::Speech);
        // Silence after fresh speech starts a new run, so it's the edge
        // event again, not a continuation of the old timer.
        assert_eq!(vad.push(&quiet(160)), VadEvent::SilenceStart);
    }
}
