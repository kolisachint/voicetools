//! Microphone capture via `cpal`.
//!
//! We stream **native-rate mono** chunks over a channel rather than resampling
//! inside the realtime audio callback. VAD runs on these chunks (RMS is
//! sample-rate independent), and the full buffer is resampled to 16 kHz once,
//! right before transcription — this avoids the phase discontinuities you'd
//! get from resampling each callback buffer independently.

use std::sync::mpsc::{channel, Receiver};

use anyhow::{anyhow, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::audio::to_mono;

/// A live capture session. Hold onto it to keep the stream running; drop it
/// (or call [`Capture::stop`]) to release the device.
pub struct Capture {
    /// Native-rate mono chunks from the input device.
    pub chunks: Receiver<Vec<f32>>,
    /// The device's sample rate; resample the captured buffer from this.
    pub input_rate: u32,
    stream: Stream,
}

impl Capture {
    /// Explicitly stop and release the input stream.
    pub fn stop(self) {
        drop(self.stream);
    }
}

/// Open the default input device and begin streaming mono chunks.
pub fn start() -> anyhow::Result<Capture> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("no default input device — is a microphone connected?")?;

    let config = device
        .default_input_config()
        .context("failed to read default input config")?;
    let input_rate = config.sample_rate().0;
    let channels = config.channels();
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let (tx, rx) = channel::<Vec<f32>>();
    let err_fn = |e| eprintln!("[mic] stream error: {e}");

    // Each branch converts the device's native sample type to f32, downmixes
    // to mono, and forwards the chunk. Send errors mean the receiver hung up,
    // which simply ends capture — so they're ignored.
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| {
                let _ = tx.send(to_mono(data, channels));
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                let f: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                let _ = tx.send(to_mono(&f, channels));
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                let f: Vec<f32> = data
                    .iter()
                    .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                    .collect();
                let _ = tx.send(to_mono(&f, channels));
            },
            err_fn,
            None,
        ),
        other => return Err(anyhow!("unsupported sample format: {other:?}")),
    }
    .context("failed to build input stream")?;

    stream.play().context("failed to start input stream")?;

    Ok(Capture {
        chunks: rx,
        input_rate,
        stream,
    })
}
