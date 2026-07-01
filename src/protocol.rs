//! The stdout line protocol spoken to whatever drives `voicetools`
//! (e.g. the hoocode TUI). Exactly one event per line on **stdout**.
//! **stderr** is free for human-readable debug logs and download progress,
//! so a consumer can parse stdout without seeing noise.
//!
//! ```text
//! STATUS recording        # state transitions: recording | transcribing | ...
//! SEGMENT hello world      # a chunk of decoded text (may be a single word)
//! DONE                     # transcription finished successfully
//! ERROR no model found     # fatal error; process will exit non-zero
//! ```
//!
//! `voicetools serve` (see `src/serve.rs`) speaks the same protocol plus
//! three daemon-only lines:
//!
//! ```text
//! READY                    # models finished loading; ready for START
//! LEVEL 0.0123              # live RMS energy for one audio chunk
//! PHASE silence             # trailing silence just started
//! ```

use std::io::Write;

/// Emit a state transition (e.g. `recording`, `transcribing`).
pub fn status(s: &str) {
    println!("STATUS {s}");
    flush();
}

/// Emit once, after `serve` finishes loading models and can accept `START`.
pub fn ready() {
    println!("READY");
    flush();
}

/// Emit once per audio chunk while `serve` is listening, so a UI can draw a
/// live level meter. `rms` is the chunk's root-mean-square amplitude.
pub fn level(rms: f32) {
    println!("LEVEL {rms}");
    flush();
}

/// Emit a lightweight phase marker within a listening session (currently
/// just `silence`, when trailing silence begins).
pub fn phase(s: &str) {
    println!("PHASE {s}");
    flush();
}

/// Emit a chunk of recognized text. Consumers typically append these
/// to the input buffer as they stream in.
pub fn segment(s: &str) {
    println!("SEGMENT {s}");
    flush();
}

/// Signal successful completion.
pub fn done() {
    println!("DONE");
    flush();
}

/// Emit a fatal error line. The caller is responsible for exiting.
pub fn error(s: &str) {
    println!("ERROR {s}");
    flush();
}

/// Flush stdout so line-oriented consumers see events immediately rather
/// than at buffer boundaries. The protocol is useless if it's not live.
fn flush() {
    let _ = std::io::stdout().flush();
}
