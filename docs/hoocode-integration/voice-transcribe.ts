// packages/tui/src/voice-transcribe.ts
//
// Keeps a `voicetools serve` daemon alive for the life of the TUI process,
// so push-to-talk has no per-press model load. Writes START/CANCEL to its
// stdin and parses the extended stdout line protocol
// (READY/STATUS/LEVEL/PHASE/SEGMENT/DONE/ERROR); recognized text is fed into
// the TUI input via bracketed paste. stderr is ignored (it carries the
// binary's own debug logs).
//
// Binaries older than the `serve` subcommand fall back to spawning
// `voicetools transcribe` per press, matching the previous behavior.

import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";
import type { StdinBuffer } from "./stdin-buffer.js";

export type VoiceEvent =
  | { type: "warming" }
  | { type: "ready" }
  | { type: "listening" }
  | { type: "level"; rms: number }
  | { type: "silence" }
  | { type: "transcribing" }
  | { type: "done" }
  | { type: "error"; message: string };

export interface VoiceSession {
  /** Stop the running transcription early. */
  cancel(): void;
}

const RESPAWN_DELAY_MS = 500;
const SHUTDOWN_GRACE_MS = 500;

/**
 * Long-lived client for `voicetools serve`. Create one instance for the
 * process lifetime; call `startCapture()` per push-to-talk press and
 * `dispose()` on TUI exit.
 */
export class VoiceDaemon {
  private proc: ChildProcessWithoutNullStreams | null = null;
  private serveSupported: boolean | null = null;
  private warmedUp = false;
  private disposed = false;
  private fallback: VoiceSession | null = null;

  constructor(
    private readonly bin: string,
    private readonly stdinBuffer: StdinBuffer,
    private readonly onEvent: (e: VoiceEvent) => void,
  ) {}

  /** Begin a push-to-talk capture, spawning/probing the daemon on first use. */
  async startCapture(): Promise<VoiceSession> {
    if (this.serveSupported === null) {
      this.serveSupported = await probeServeSupport(this.bin);
    }

    if (!this.serveSupported) {
      this.fallback = transcribeToInput(this.bin, this.stdinBuffer, (s) =>
        this.onEvent(legacyStatusToEvent(s)),
      );
      return this.fallback;
    }

    if (!this.proc) this.spawnDaemon();
    this.proc?.stdin.write("START\n");
    return { cancel: () => this.proc?.stdin.write("CANCEL\n") };
  }

  /** Stop the daemon for good; call once, on TUI exit. */
  dispose(): void {
    this.disposed = true;
    this.fallback?.cancel();
    const proc = this.proc;
    if (!proc) return;
    proc.stdin.write("SHUTDOWN\n");
    setTimeout(() => proc.kill(), SHUTDOWN_GRACE_MS);
  }

  private spawnDaemon(): void {
    const proc = spawn(this.bin, ["serve"], { stdio: ["pipe", "pipe", "ignore"] });
    this.proc = proc;
    if (!this.warmedUp) this.onEvent({ type: "warming" });

    createInterface({ input: proc.stdout }).on("line", (line) => this.handleLine(line));
    proc.on("error", (e) => this.onEvent({ type: "error", message: e.message }));
    proc.on("exit", () => {
      this.proc = null;
      if (this.disposed) return;
      // Crash recovery: respawn so the next press doesn't fall silent. The
      // next `startCapture()` write races the respawn harmlessly — worst
      // case one press is missed while the new daemon boots.
      setTimeout(() => this.spawnDaemon(), RESPAWN_DELAY_MS);
    });
  }

  private handleLine(line: string): void {
    if (line === "READY") {
      this.warmedUp = true;
      this.onEvent({ type: "ready" });
    } else if (line === "STATUS listening") {
      this.onEvent({ type: "listening" });
    } else if (line === "STATUS transcribing") {
      this.onEvent({ type: "transcribing" });
    } else if (line.startsWith("LEVEL ")) {
      const rms = Number(line.slice("LEVEL ".length));
      if (!Number.isNaN(rms)) this.onEvent({ type: "level", rms });
    } else if (line === "PHASE silence") {
      this.onEvent({ type: "silence" });
    } else if (line.startsWith("SEGMENT ")) {
      const text = line.slice("SEGMENT ".length);
      // Trailing space separates streamed words.
      this.stdinBuffer.process(`\x1b[200~${text} \x1b[201~`);
    } else if (line === "DONE") {
      this.onEvent({ type: "done" });
    } else if (line.startsWith("ERROR ")) {
      this.onEvent({ type: "error", message: line.slice("ERROR ".length) });
    }
  }
}

/** `voicetools serve --help` succeeds (exit 0) iff the subcommand exists, regardless of model-load time — cheap and doesn't wait for READY. */
function probeServeSupport(bin: string): Promise<boolean> {
  return new Promise((resolve) => {
    const proc = spawn(bin, ["serve", "--help"], { stdio: "ignore" });
    proc.on("error", () => resolve(false));
    proc.on("exit", (code) => resolve(code === 0));
  });
}

function legacyStatusToEvent(status: string): VoiceEvent {
  if (status === "recording") return { type: "listening" };
  if (status === "transcribing") return { type: "transcribing" };
  if (status.startsWith("error")) return { type: "error", message: status };
  return { type: "ready" };
}

/** Pre-`serve` fallback: spawns `voicetools transcribe` for a single press. */
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

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const METER_BARS = "▁▂▃▄▅▆▇█";
/** Roughly the loudest RMS a speech chunk hits; used to scale the meter. */
const METER_FULL_SCALE_RMS = 0.3;

/**
 * Tracks `VoiceEvent`s and renders a single compact status line, so the TUI
 * doesn't need its own state machine. Call `handle()` as events arrive and
 * `render()` on each redraw (e.g. every 100ms via `setInterval` while
 * `active` is true, for the spinner/meter/countdown to animate).
 */
export class VoiceStatusLine {
  private phase: "idle" | "warming" | "listening" | "silence" | "transcribing" = "idle";
  private level = 0;
  private silenceStartedAt = 0;

  constructor(private readonly silenceMs = 600) {}

  /** Whether the caller should keep re-rendering (redraw loop can stop otherwise). */
  get active(): boolean {
    return this.phase !== "idle";
  }

  handle(event: VoiceEvent): void {
    switch (event.type) {
      case "warming":
        this.phase = "warming";
        break;
      case "ready":
        // Only the boot-time warmup shows text; later presses jump straight
        // to "listening" via the `listening` event below.
        if (this.phase === "warming") this.phase = "idle";
        break;
      case "listening":
        this.phase = "listening";
        this.level = 0;
        break;
      case "level":
        this.level = event.rms;
        break;
      case "silence":
        this.phase = "silence";
        this.silenceStartedAt = Date.now();
        break;
      case "transcribing":
        this.phase = "transcribing";
        break;
      case "done":
      case "error":
        this.phase = "idle"; // collapse
        break;
    }
  }

  render(): string {
    switch (this.phase) {
      case "warming":
        return `${spinnerFrame()} Warming…`;
      case "listening":
        return `🔴 Listening ${meterBar(this.level)}`;
      case "silence": {
        const remainingMs = Math.max(0, this.silenceMs - (Date.now() - this.silenceStartedAt));
        return `🔴 Listening ${meterBar(this.level)}  ${(remainingMs / 1000).toFixed(1)}s`;
      }
      case "transcribing":
        return "✦ Transcribing…";
      default:
        return "";
    }
  }
}

function spinnerFrame(): string {
  return SPINNER_FRAMES[Math.floor(Date.now() / 80) % SPINNER_FRAMES.length];
}

function meterBar(rms: number): string {
  const idx = Math.min(
    METER_BARS.length - 1,
    Math.max(0, Math.floor((rms / METER_FULL_SCALE_RMS) * METER_BARS.length)),
  );
  return METER_BARS[idx];
}
