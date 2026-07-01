//! Whisper.cpp fallback backend via `whisper-rs`.
//!
//! Enabled with `--features whisper`. Slower and English-only at the
//! `small.en` size, but a dependency-light fallback when Parakeet/ONNX isn't
//! an option. Segments are emitted as whisper.cpp produces them.

use std::path::Path;

use anyhow::Context;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::Transcriber;

pub struct WhisperTranscriber {
    ctx: WhisperContext,
}

impl WhisperTranscriber {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let model = dir.join("ggml-small.en.bin");
        let ctx = WhisperContext::new_with_params(
            model.to_string_lossy().as_ref(),
            WhisperContextParameters::default(),
        )
        .with_context(|| format!("loading whisper model {}", model.display()))?;
        Ok(Self { ctx })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, pcm: &[f32], on_segment: &mut dyn FnMut(&str)) -> anyhow::Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let mut state = self.ctx.create_state().context("creating whisper state")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, pcm)
            .context("whisper inference failed")?;

        let n = state
            .full_n_segments()
            .context("counting whisper segments")?;
        for i in 0..n {
            let text = state
                .full_get_segment_text(i)
                .context("reading whisper segment")?;
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                on_segment(trimmed);
            }
        }
        Ok(())
    }
}
