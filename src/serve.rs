//! `voicetools serve`: load models once, then answer START/CANCEL/SHUTDOWN
//! commands on stdin with the same stdout line protocol as `transcribe`,
//! plus READY/LEVEL/PHASE for a long-lived UI. See `src/protocol.rs`.
//!
//! Two threads are involved: this one owns the mic/VAD/transcribe pipeline;
//! a second reads stdin lines and forwards parsed commands over a channel,
//! so a blocking `stdin.lines()` read never stalls audio capture or a
//! pending CANCEL.

use std::io::BufRead;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crate::setup::Model;
use crate::transcribe::Transcriber;
use crate::vad::{self, Vad, VadEvent};
use crate::{audio, mic, protocol};

/// How often to poll the audio channel while listening, so CANCEL/SHUTDOWN
/// are noticed promptly even during silence.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

enum Ctrl {
    Start,
    Cancel,
    Shutdown,
}

/// Run the daemon: load `model_name`, emit READY, then loop on stdin
/// commands until SHUTDOWN or stdin closes.
pub fn run(model_name: &str, silence_ms: u64) -> anyhow::Result<()> {
    let model = Model::parse(model_name)?;
    if !model.is_ready() {
        protocol::error(&format!(
            "no model found for '{}' — run: voicetools setup --model {}",
            model.id(),
            model.id()
        ));
        std::process::exit(1);
    }

    let mut transcriber = crate::transcribe::load(model)?;
    protocol::ready();

    let cmds = spawn_stdin_reader();

    loop {
        match cmds.recv() {
            Ok(Ctrl::Start) => match listen(&mut *transcriber, silence_ms, &cmds) {
                Ok(Signal::Shutdown) => break,
                Ok(Signal::Continue) => {}
                Err(e) => protocol::error(&format!("{e:#}")),
            },
            Ok(Ctrl::Cancel) => {} // nothing is running; ignore
            Ok(Ctrl::Shutdown) | Err(_) => break,
        }
    }
    Ok(())
}

enum Signal {
    Continue,
    Shutdown,
}

/// Capture + VAD one utterance and transcribe it, reacting to CANCEL/
/// SHUTDOWN while listening. One capture at a time; the mic is opened here
/// and closed before returning.
fn listen(
    transcriber: &mut dyn Transcriber,
    silence_ms: u64,
    cmds: &Receiver<Ctrl>,
) -> anyhow::Result<Signal> {
    let capture = mic::start()?;
    let input_rate = capture.input_rate;
    let mut vad = Vad::new(silence_ms);
    let mut raw: Vec<f32> = Vec::new();
    let mut cancelled = false;

    protocol::status("listening");

    loop {
        match capture.chunks.recv_timeout(POLL_INTERVAL) {
            Ok(chunk) => {
                protocol::level(vad::rms(&chunk));
                let event = vad.push(&chunk);
                raw.extend_from_slice(&chunk);
                match event {
                    VadEvent::SilenceStart => protocol::phase("silence"),
                    VadEvent::SilenceTimeout => break,
                    VadEvent::Speech | VadEvent::Silence => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Sender dropped: device error/disconnect ends capture.
            Err(RecvTimeoutError::Disconnected) => break,
        }

        match cmds.try_recv() {
            Ok(Ctrl::Cancel) => {
                cancelled = true;
                break;
            }
            Ok(Ctrl::Shutdown) => {
                capture.stop();
                return Ok(Signal::Shutdown);
            }
            Ok(Ctrl::Start) => {} // already listening; ignore duplicate START
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                capture.stop();
                return Ok(Signal::Shutdown);
            }
        }
    }
    capture.stop();

    // A cancelled capture is discarded, not transcribed.
    if cancelled || !vad.heard_speech() {
        protocol::done();
        return Ok(Signal::Continue);
    }

    protocol::status("transcribing");
    let pcm = audio::resample_to_16k(&raw, input_rate);
    let mut emit = |seg: &str| protocol::segment(seg);
    transcriber.transcribe(&pcm, &mut emit)?;
    protocol::done();
    Ok(Signal::Continue)
}

/// Read newline-delimited commands from stdin on a background thread and
/// forward them over a channel, so the mic/VAD loop never blocks on stdin.
fn spawn_stdin_reader() -> Receiver<Ctrl> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let cmd = match line.trim() {
                "START" => Ctrl::Start,
                "CANCEL" => Ctrl::Cancel,
                "SHUTDOWN" => Ctrl::Shutdown,
                "" => continue,
                other => {
                    eprintln!("[serve] unknown command: {other}");
                    continue;
                }
            };
            let is_shutdown = matches!(cmd, Ctrl::Shutdown);
            if tx.send(cmd).is_err() || is_shutdown {
                break;
            }
        }
    });
    rx
}
