// packages/tui/src/voice-transcribe.ts
//
// Spawns `voicetools transcribe`, parses its stdout line protocol, and feeds
// recognized text into the TUI input via bracketed paste. stderr is ignored
// (it carries the binary's own debug logs).

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import type { StdinBuffer } from "./stdin-buffer.js";

export interface VoiceSession {
  /** Stop the running transcription early. */
  cancel(): void;
}

export function transcribeToInput(
  bin: string,
  stdinBuffer: StdinBuffer,
  onStatus: (s: string) => void,
): VoiceSession {
  const proc = spawn(bin, ["transcribe"], {
    stdio: ["ignore", "pipe", "ignore"],
  });

  const rl = createInterface({ input: proc.stdout });

  rl.on("line", (line) => {
    if (line.startsWith("STATUS ")) {
      onStatus(line.slice("STATUS ".length));
      return;
    }
    if (line.startsWith("SEGMENT ")) {
      const text = line.slice("SEGMENT ".length);
      // Inject via bracketed paste — StdinBuffer's native paste path, so no
      // extra handling is needed. Trailing space separates streamed words.
      stdinBuffer.process(`\x1b[200~${text} \x1b[201~`);
      return;
    }
    if (line === "DONE") {
      proc.kill();
      return;
    }
    if (line.startsWith("ERROR ")) {
      onStatus(`error: ${line.slice("ERROR ".length)}`);
      proc.kill();
      return;
    }
  });

  proc.on("error", (e) => onStatus(`error: ${e.message}`));

  return {
    cancel() {
      proc.kill();
    },
  };
}
