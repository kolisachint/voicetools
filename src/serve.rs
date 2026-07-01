//! `voicetools serve`: load models once, then answer START/CANCEL/SHUTDOWN
//! commands on stdin with the stdout line protocol. See `src/protocol.rs`.
//!
//! Unlike the batch `transcribe` subcommand — which records the whole
//! utterance and only decodes after you stop — `serve` transcribes **while
//! you speak**: on a short interval it re-runs the pipeline over the audio
//! captured so far and emits a `PARTIAL` line, so text appears live. When the
//! utterance ends it emits one authoritative `FINAL`, then `DONE`. That
//! rolling re-decode is what gives a UI real content to show.
//!
//! Two threads are involved: this one owns the mic/VAD/transcribe pipeline;
//! a second reads stdin lines and forwards parsed commands over a channel,
//! so a blocking `stdin.lines()` read never stalls audio capture or a
//! pending CANCEL.

use std::io::BufRead;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

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
/// commands until SHUTDOWN or stdin closes. `partial_ms` is the minimum gap
/// between live `PARTIAL` re-decodes while listening; `0` disables partials
/// (final-only, like the batch path).
pub fn run(model_name: &str, silence_ms: u64, partial_ms: u64) -> anyhow::Result<()> {
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
            Ok(Ctrl::Start) => match listen(&mut *transcriber, silence_ms, partial_ms, &cmds) {
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

/// Capture + VAD one utterance, streaming `PARTIAL`s while listening and a
/// `FINAL` at the end, reacting to CANCEL/SHUTDOWN throughout. One capture at
/// a time; the mic is opened here and closed before returning.
fn listen(
    transcriber: &mut dyn Transcriber,
    silence_ms: u64,
    partial_ms: u64,
    cmds: &Receiver<Ctrl>,
) -> anyhow::Result<Signal> {
    let capture = mic::start()?;
    let input_rate = capture.input_rate;
    let mut vad = Vad::new(silence_ms);
    let mut raw: Vec<f32> = Vec::new();
    let mut cancelled = false;
    let mut stop = false;

    // Streaming-partial bookkeeping: don't re-decode the same audio twice,
    // and hold off until the next allowed slot.
    let mut last_decoded_len = 0usize;
    let mut next_partial = Instant::now() + Duration::from_millis(partial_ms);

    protocol::status("listening");

    while !stop {
        match capture.chunks.recv_timeout(POLL_INTERVAL) {
            Ok(chunk) => {
                protocol::level(vad::rms(&chunk));
                match vad.push(&chunk) {
                    VadEvent::SilenceStart => protocol::phase("silence"),
                    VadEvent::SilenceTimeout => stop = true,
                    VadEvent::Speech | VadEvent::Silence => {}
                }
                raw.extend_from_slice(&chunk);
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Sender dropped: device error/disconnect ends capture.
            Err(RecvTimeoutError::Disconnected) => stop = true,
        }

        match cmds.try_recv() {
            Ok(Ctrl::Cancel) => {
                cancelled = true;
                stop = true;
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

        // Live partial: only after real speech, only when new audio arrived,
        // and only once the interval has elapsed. The next slot is pushed out
        // by however long the decode took, so a slow decode throttles itself
        // rather than starving the capture-drain loop.
        if partial_ms > 0
            && !stop
            && vad.heard_speech()
            && raw.len() > last_decoded_len
            && Instant::now() >= next_partial
        {
            let started = Instant::now();
            last_decoded_len = raw.len();
            let pcm = audio::resample_to_16k(&raw, input_rate);
            match transcribe_to_string(transcriber, &pcm) {
                Ok(text) if !text.is_empty() => protocol::partial(&text),
                Ok(_) => {}
                Err(e) => protocol::error(&format!("{e:#}")),
            }
            let interval = Duration::from_millis(partial_ms);
            next_partial = Instant::now() + interval.max(started.elapsed());
        }
    }
    capture.stop();

    // A cancelled capture is discarded; so is one that never heard speech.
    if cancelled || !vad.heard_speech() {
        protocol::done();
        return Ok(Signal::Continue);
    }

    protocol::status("transcribing");
    let pcm = audio::resample_to_16k(&raw, input_rate);
    let text = transcribe_to_string(transcriber, &pcm)?;
    protocol::final_text(&text);
    protocol::done();
    Ok(Signal::Continue)
}

/// Run the transcriber over `pcm` and collect its streamed word segments into
/// one space-joined string — the form both `PARTIAL` and `FINAL` want.
fn transcribe_to_string(transcriber: &mut dyn Transcriber, pcm: &[f32]) -> anyhow::Result<String> {
    let mut text = String::new();
    let mut push = |seg: &str| {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(seg);
    };
    transcriber.transcribe(pcm, &mut push)?;
    Ok(text)
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
