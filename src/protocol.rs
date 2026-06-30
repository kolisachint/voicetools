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

use std::io::Write;

/// Emit a state transition (e.g. `recording`, `transcribing`).
pub fn status(s: &str) {
    println!("STATUS {s}");
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
