//! voicetools — local, offline voice-to-text for the terminal.
//!
//! Subcommands:
//!   * `setup`      — download a model (first-run wizard)
//!   * `models`     — list installed models
//!   * `transcribe` — capture from the mic (or `--wav`) and stream text
//!
//! With no subcommand, `transcribe` runs. All output the consumer cares about
//! goes to **stdout** as the line protocol in [`protocol`]; logs go to stderr.

mod audio;
mod mic;
mod protocol;
mod setup;
mod tls;
mod transcribe;
mod vad;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use setup::Model;
use vad::{Vad, VadEvent};

#[derive(Parser)]
#[command(name = "voicetools", version, about = "Local offline voice-to-text")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// First-run setup: download a model.
    Setup {
        #[arg(long, default_value = "parakeet-v3")]
        model: String,
        /// Extra CA certificate (PEM) to trust, in addition to the OS store.
        /// Repeatable.
        #[arg(long = "ca-cert")]
        ca_cert: Vec<PathBuf>,
        /// Disable TLS certificate verification for downloads. Dangerous —
        /// only use on a trusted network as a last resort.
        #[arg(long)]
        insecure: bool,
    },
    /// List known models and whether they're installed.
    Models,
    /// Transcribe from the microphone (default) or a WAV file.
    Transcribe(TranscribeArgs),
}

#[derive(Args, Clone)]
struct TranscribeArgs {
    /// Model to use (parakeet-v3, parakeet-v2, whisper-small).
    #[arg(long, env = "VOICETOOLS_MODEL", default_value = "parakeet-v3")]
    model: String,
    /// Trailing-silence timeout, in milliseconds, before auto-stop.
    #[arg(long, env = "VOICE_SILENCE_MS", default_value_t = 600)]
    silence_ms: u64,
    /// Transcribe this WAV file instead of capturing from the mic.
    #[arg(long)]
    wav: Option<PathBuf>,
}

impl TranscribeArgs {
    /// Defaults used when `transcribe` runs as the implicit command, honoring
    /// the same env vars clap would.
    fn from_env() -> Self {
        let model = std::env::var("VOICETOOLS_MODEL").unwrap_or_else(|_| "parakeet-v3".into());
        let silence_ms = std::env::var("VOICE_SILENCE_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600);
        Self {
            model,
            silence_ms,
            wav: None,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Setup {
            model,
            ca_cert,
            insecure,
        }) => setup::run(
            &model,
            &tls::TlsOptions {
                extra_ca_certs: ca_cert,
                insecure,
            },
        ),
        Some(Command::Models) => setup::list(),
        Some(Command::Transcribe(args)) => run_transcribe(args),
        None => run_transcribe(TranscribeArgs::from_env()),
    };

    if let Err(e) = result {
        protocol::error(&format!("{e:#}"));
        std::process::exit(1);
    }
}

fn run_transcribe(args: TranscribeArgs) -> anyhow::Result<()> {
    let model = Model::parse(&args.model)?;
    if !model.is_ready() {
        protocol::error(&format!(
            "no model found for '{}' — run: voicetools setup --model {}",
            model.id(),
            model.id()
        ));
        std::process::exit(1);
    }

    let mut transcriber = transcribe::load(model)?;
    let mut emit = |seg: &str| protocol::segment(seg);

    // File mode: handy for validating the inference path without a mic.
    if let Some(path) = &args.wav {
        protocol::status("transcribing");
        let pcm = audio::load_wav_16k_mono(path)?;
        transcriber.transcribe(&pcm, &mut emit)?;
        protocol::done();
        return Ok(());
    }

    // Mic mode: capture native-rate chunks, stop on trailing silence, then
    // resample the whole buffer to 16 kHz for transcription.
    let capture = mic::start()?;
    let input_rate = capture.input_rate;
    let mut vad = Vad::new(args.silence_ms);
    let mut raw: Vec<f32> = Vec::new();

    protocol::status("recording");
    // Loop ends on trailing-silence timeout, or when the sender drops (device
    // error / disconnect), which makes `recv` return `Err`.
    while let Ok(chunk) = capture.chunks.recv() {
        let event = vad.push(&chunk);
        raw.extend_from_slice(&chunk);
        if event == VadEvent::SilenceTimeout {
            break;
        }
    }
    capture.stop();

    if !vad.heard_speech() {
        protocol::status("transcribing");
        protocol::done();
        return Ok(());
    }

    protocol::status("transcribing");
    let pcm = audio::resample_to_16k(&raw, input_rate);
    transcriber.transcribe(&pcm, &mut emit)?;
    protocol::done();
    Ok(())
}
