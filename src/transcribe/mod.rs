//! Transcription backends behind a single trait, so the mic/VAD pipeline
//! doesn't care whether Parakeet or Whisper is doing the work.

use crate::setup::{Backend, Model};

#[cfg(feature = "parakeet")]
pub mod parakeet;
#[cfg(feature = "whisper")]
pub mod whisper;

/// A speech-to-text backend.
pub trait Transcriber {
    /// Transcribe 16 kHz mono f32 PCM. `on_segment` is called for each chunk
    /// of recognized text (typically a word) as it is decoded, so callers can
    /// stream output rather than waiting for the whole utterance.
    fn transcribe(
        &mut self,
        pcm: &[f32],
        on_segment: &mut dyn FnMut(&str),
    ) -> anyhow::Result<()>;
}

/// Load the transcriber for `model`, picking the backend by feature flags.
///
/// Returns a clear error if the required backend wasn't compiled in, so a
/// `--no-default-features` build fails loudly rather than silently.
pub fn load(model: Model) -> anyhow::Result<Box<dyn Transcriber>> {
    let dir = model.dir()?;
    let _ = &dir; // consumed by the feature-gated arms below
    match model.backend() {
        Backend::Parakeet => {
            #[cfg(feature = "parakeet")]
            {
                Ok(Box::new(parakeet::ParakeetTranscriber::load(&dir)?))
            }
            #[cfg(not(feature = "parakeet"))]
            {
                anyhow::bail!("this build was compiled without the `parakeet` feature")
            }
        }
        Backend::Whisper => {
            #[cfg(feature = "whisper")]
            {
                Ok(Box::new(whisper::WhisperTranscriber::load(&dir)?))
            }
            #[cfg(not(feature = "whisper"))]
            {
                anyhow::bail!("this build was compiled without the `whisper` feature")
            }
        }
    }
}
