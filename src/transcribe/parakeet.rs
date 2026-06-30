//! Parakeet-TDT inference via ONNX Runtime (`ort`).
//!
//! Pipeline: `nemo128.onnx` turns the waveform into a 128-bin mel
//! spectrogram, `encoder.int8.onnx` encodes it, and the combined
//! `decoder_joint.int8.onnx` runs the predictor + joint network for greedy
//! Token-and-Duration-Transducer (TDT) decoding.
//!
//! The greedy decode (duration jumps, blank handling) follows the reference
//! implementation in `jason-ni/parakeet-rs`. The difference here is that the
//! predictor and joint are **fused** into one `decoder_joint` session (the
//! `istupakov` / PalatineVision export the `setup` command downloads), so each
//! step is a single session call.
//!
//! ## Model I/O names
//!
//! The input/output tensor names below match the istupakov/PalatineVision
//! ONNX export. If you bring a model with different names, inspect it with a
//! tool like [Netron](https://netron.app) and adjust the `const`s — that's the
//! only thing that's export-specific; the decode logic is not.

use std::path::Path;

use anyhow::{anyhow, Context};
use ndarray::{s, ArrayD, ArrayViewD, IxDyn};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::{Tensor, TensorRef};

use super::Transcriber;

// --- Preprocessor (nemo128.onnx) I/O ---
const PRE_WAVE_IN: &str = "waveforms";
const PRE_WAVE_LEN_IN: &str = "waveforms_lens";
const PRE_FEAT_OUT: &str = "features";

// --- Encoder (encoder.int8.onnx) I/O ---
const ENC_SIGNAL_IN: &str = "audio_signal";
const ENC_LEN_IN: &str = "length";
const ENC_OUT: &str = "outputs";

// --- Decoder+Joint (decoder_joint.int8.onnx) I/O ---
const DJ_ENC_IN: &str = "encoder_outputs";
const DJ_TARGETS_IN: &str = "targets";
const DJ_TARGET_LEN_IN: &str = "target_length";
const DJ_STATE1_IN: &str = "input_states_1";
const DJ_STATE2_IN: &str = "input_states_2";
const DJ_LOGITS_OUT: &str = "outputs";
const DJ_STATE1_OUT: &str = "output_states_1";
const DJ_STATE2_OUT: &str = "output_states_2";

/// Number of TDT duration classes (durations 0..=4). The joint output is
/// `[token_logits .. duration_logits]`; this is the size of the trailing
/// duration block, used to split the two.
const NUM_DURATIONS: usize = 5;

/// LSTM predictor state shape `[layers, batch, hidden]`.
const STATE_SHAPE: [usize; 3] = [2, 1, 640];

/// Safety cap on non-blank symbols emitted at a single encoder frame, to
/// guarantee the decode loop terminates.
const MAX_SYMBOLS_PER_FRAME: usize = 10;

/// Subword marker prefix that denotes the start of a new word (U+2581).
const WORD_PREFIX: char = '▁';

pub struct ParakeetTranscriber {
    preprocessor: Session,
    encoder: Session,
    decoder_joint: Session,
    vocab: Vec<String>,
}

impl ParakeetTranscriber {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let preprocessor = build_session(&dir.join("nemo128.onnx"))?;
        let encoder = build_session(&dir.join("encoder.int8.onnx"))?;
        let decoder_joint = build_session(&dir.join("decoder_joint.int8.onnx"))?;
        let vocab = load_vocab(&dir.join("vocab.txt"))?;
        Ok(Self {
            preprocessor,
            encoder,
            decoder_joint,
            vocab,
        })
    }

    /// Waveform -> mel features `[1, 128, T]`.
    fn extract_features(&mut self, pcm: &[f32]) -> anyhow::Result<ArrayD<f32>> {
        let audio = ArrayViewD::from_shape(IxDyn(&[1, pcm.len()]), pcm)
            .context("shaping pcm for preprocessor")?;
        let inputs = ort::inputs![
            PRE_WAVE_IN => TensorRef::from_array_view(audio.view())?,
            PRE_WAVE_LEN_IN => Tensor::from_array(([1], vec![pcm.len() as i64].into_boxed_slice()))?,
        ];
        let outputs = self.preprocessor.run(inputs)?;
        let feats: ArrayViewD<f32> = outputs
            .get(PRE_FEAT_OUT)
            .ok_or_else(|| missing(PRE_FEAT_OUT, "preprocessor"))?
            .try_extract_array()?;
        Ok(feats.to_owned())
    }

    /// Mel features -> encoder output `[1, D, T']` (channels-first).
    fn encode(&mut self, feats: &ArrayD<f32>) -> anyhow::Result<ArrayD<f32>> {
        let length = feats.shape()[2] as i64;
        let inputs = ort::inputs![
            ENC_SIGNAL_IN => TensorRef::from_array_view(feats.view())?,
            ENC_LEN_IN => Tensor::from_array(([1], vec![length].into_boxed_slice()))?,
        ];
        let outputs = self.encoder.run(inputs)?;
        let enc: ArrayViewD<f32> = outputs
            .get(ENC_OUT)
            .ok_or_else(|| missing(ENC_OUT, "encoder"))?
            .try_extract_array()?;
        Ok(enc.to_owned())
    }

    /// One predictor+joint step over a single encoder frame.
    ///
    /// Returns the joint logits (length `vocab + NUM_DURATIONS`) and the next
    /// LSTM states.
    fn decoder_joint_step(
        &mut self,
        enc_frame: &ArrayD<f32>,
        label: i32,
        state1: &ArrayD<f32>,
        state2: &ArrayD<f32>,
    ) -> anyhow::Result<(Vec<f32>, ArrayD<f32>, ArrayD<f32>)> {
        let inputs = ort::inputs![
            DJ_ENC_IN => TensorRef::from_array_view(enc_frame.view())?,
            DJ_TARGETS_IN => Tensor::from_array(([1, 1], vec![label].into_boxed_slice()))?,
            DJ_TARGET_LEN_IN => Tensor::from_array(([1], vec![1i32].into_boxed_slice()))?,
            DJ_STATE1_IN => TensorRef::from_array_view(state1.view())?,
            DJ_STATE2_IN => TensorRef::from_array_view(state2.view())?,
        ];
        let outputs = self.decoder_joint.run(inputs)?;

        let logits: Vec<f32> = outputs
            .get(DJ_LOGITS_OUT)
            .ok_or_else(|| missing(DJ_LOGITS_OUT, "decoder_joint"))?
            .try_extract_array::<f32>()?
            .iter()
            .copied()
            .collect();
        let next1: ArrayD<f32> = outputs
            .get(DJ_STATE1_OUT)
            .ok_or_else(|| missing(DJ_STATE1_OUT, "decoder_joint"))?
            .try_extract_array::<f32>()?
            .to_owned();
        let next2: ArrayD<f32> = outputs
            .get(DJ_STATE2_OUT)
            .ok_or_else(|| missing(DJ_STATE2_OUT, "decoder_joint"))?
            .try_extract_array::<f32>()?
            .to_owned();
        Ok((logits, next1, next2))
    }
}

impl Transcriber for ParakeetTranscriber {
    fn transcribe(
        &mut self,
        pcm: &[f32],
        on_segment: &mut dyn FnMut(&str),
    ) -> anyhow::Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let feats = self.extract_features(pcm)?;
        let enc = self.encode(&feats)?;

        // Encoder output is channels-first: [1, D, T'].
        let d = enc.shape()[1];
        let n_frames = enc.shape()[2];
        if n_frames == 0 {
            return Ok(());
        }

        let mut state1: ArrayD<f32> = ArrayD::zeros(IxDyn(&STATE_SHAPE));
        let mut state2: ArrayD<f32> = ArrayD::zeros(IxDyn(&STATE_SHAPE));

        // RNNT/TDT start-of-sequence is the blank id. We don't know the exact
        // vocab/blank split until we've seen the joint output, so seed with the
        // last vocab index (blank is conventionally last) and refine per step.
        let mut label: i32 = self.vocab.len().saturating_sub(1) as i32;

        let mut t = 0usize;
        let mut symbols_at_t = 0usize;
        let mut word = String::new();

        while t < n_frames {
            // Single encoder frame as [1, D, 1], standard layout.
            let frame: ArrayD<f32> = enc.slice(s![.., .., t..=t]).to_owned().into_dyn();
            let (logits, next1, next2) =
                self.decoder_joint_step(&frame, label, &state1, &state2)?;

            // Split joint output into token logits and duration logits.
            if logits.len() <= NUM_DURATIONS {
                return Err(anyhow!(
                    "decoder_joint produced {} logits, expected > {NUM_DURATIONS}",
                    logits.len()
                ));
            }
            let token_classes = logits.len() - NUM_DURATIONS;
            let blank = token_classes - 1;

            let token = argmax(&logits[..token_classes]);
            let duration = argmax(&logits[token_classes..]);

            if token != blank {
                emit_token(self.vocab.get(token).map(String::as_str), &mut word, on_segment);
                label = token as i32;
                state1 = next1;
                state2 = next2;
                symbols_at_t += 1;
            }

            // TDT time advance. A blank with zero duration would stall, so we
            // force a one-frame advance; likewise cap symbols per frame.
            let mut step = duration;
            if token == blank && step == 0 {
                step = 1;
            }
            if step > 0 {
                t += step;
                symbols_at_t = 0;
            } else if symbols_at_t >= MAX_SYMBOLS_PER_FRAME {
                t += 1;
                symbols_at_t = 0;
            }
        }

        if !word.is_empty() {
            on_segment(&word);
        }
        let _ = d; // D is documented above; referenced for clarity.
        Ok(())
    }
}

/// Append a decoded subword token to the current word, flushing the previous
/// word as a segment when a new word boundary (`▁`) is reached.
fn emit_token(token: Option<&str>, word: &mut String, on_segment: &mut dyn FnMut(&str)) {
    let Some(tok) = token else { return };
    if let Some(rest) = tok.strip_prefix(WORD_PREFIX) {
        if !word.is_empty() {
            on_segment(word);
            word.clear();
        }
        word.push_str(rest);
    } else {
        word.push_str(tok);
    }
}

/// Index of the maximum value in a slice (first on ties). Returns 0 for empty.
fn argmax(values: &[f32]) -> usize {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in values.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    best
}

/// Build a CPU ONNX session with graph optimizations enabled.
fn build_session(path: &Path) -> anyhow::Result<Session> {
    Session::builder()
        .context("creating ort session builder")?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(4)?
        .commit_from_file(path)
        .with_context(|| format!("loading model {}", path.display()))
}

/// Load `vocab.txt`: one token per line, taking the first whitespace field
/// (some exports append an index column).
fn load_vocab(path: &Path) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading vocab {}", path.display()))?;
    let vocab: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().next().unwrap_or("").to_string())
        .collect();
    if vocab.is_empty() {
        return Err(anyhow!("vocab {} is empty", path.display()));
    }
    Ok(vocab)
}

fn missing(name: &str, model: &str) -> anyhow::Error {
    anyhow!("{model} output '{name}' not found — does this ONNX export use different tensor names?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_picks_largest() {
        assert_eq!(argmax(&[0.1, 0.9, 0.3]), 1);
        assert_eq!(argmax(&[]), 0);
        assert_eq!(argmax(&[2.0, 2.0]), 0); // first on ties
    }

    #[test]
    fn emit_streams_on_word_boundary() {
        let mut out: Vec<String> = Vec::new();
        let mut word = String::new();
        {
            let mut push = |s: &str| out.push(s.to_string());
            emit_token(Some("▁hel"), &mut word, &mut push);
            emit_token(Some("lo"), &mut word, &mut push);
            emit_token(Some("▁world"), &mut word, &mut push);
        }
        // "hello" flushed when "▁world" starts; "world" still buffered.
        assert_eq!(out, vec!["hello"]);
        assert_eq!(word, "world");
    }
}
