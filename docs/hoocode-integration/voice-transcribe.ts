// packages/tui/src/voice-transcribe.ts
//
// Keeps a `voicetools serve` daemon alive for the life of the TUI process,
// so push-to-talk has no per-press model load. Writes START/CANCEL to its
// stdin and parses the streaming stdout line protocol:
//
//   READY                  models loaded; ready for START
//   STATUS listening        capture started
//   LEVEL <rms>             per-chunk mic energy (meter)
//   PARTIAL <text>          interim transcript, live as you speak (replaces)
//   PHASE silence           trailing silence started (countdown to auto-stop)
//   STATUS transcribing      final decode running
//   FINAL <text>            committed transcript (this is what gets inserted)
//   DONE                    utterance finished
//   ERROR <message>         fatal
//
// Only FINAL is injected into the TUI input (via bracketed paste). PARTIALs
// are shown live in the panel but never committed, since they change. stderr
// is ignored (the binary's own debug logs).
//
// Binaries older than `serve` fall back to spawning `voicetools transcribe`
// per press (the previous behavior).

import { spawn, type ChildProcessByStdio } from "node:child_process";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import type { StdinBuffer } from "./stdin-buffer.js";

/** `serve` process: stdin piped, stdout piped, stderr inherited/ignored. */
type ServeProcess = ChildProcessByStdio<Writable, Readable, null>;

export type VoiceEvent =
  | { type: "warming" }
  | { type: "ready" }
  | { type: "listening" }
  | { type: "level"; rms: number }
  | { type: "partial"; text: string }
  | { type: "silence" }
  | { type: "transcribing" }
  | { type: "final"; text: string }
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
  private proc: ServeProcess | null = null;
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
      // Crash recovery: respawn so the next press doesn't fall silent.
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
    } else if (line.startsWith("PARTIAL ")) {
      this.onEvent({ type: "partial", text: line.slice("PARTIAL ".length) });
    } else if (line === "PHASE silence") {
      this.onEvent({ type: "silence" });
    } else if (line.startsWith("FINAL ")) {
      const text = line.slice("FINAL ".length);
      // The one commit point: insert the finished transcript via bracketed
      // paste (StdinBuffer's native paste path). Trailing space so the next
      // dictation doesn't butt against it.
      if (text.length > 0) this.stdinBuffer.process(`\x1b[200~${text} \x1b[201~`);
      this.onEvent({ type: "final", text });
    } else if (line === "DONE") {
      this.onEvent({ type: "done" });
    } else if (line.startsWith("ERROR ")) {
      this.onEvent({ type: "error", message: line.slice("ERROR ".length) });
    }
  }
}

/** `voicetools serve --help` exits 0 iff the subcommand exists (independent of model-load time). */
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

// --- Live panel rendering --------------------------------------------------

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const METER_CELLS = 12;
/** Roughly the loudest RMS a speech chunk hits; scales the meter to full. */
const METER_FULL_SCALE_RMS = 0.3;

/**
 * Turns the `VoiceEvent` stream into a multi-line live panel — the "what's
 * happening" view. Feed it every event via `handle()`, and call `render()`
 * on each redraw (e.g. every ~80ms while `active`, so the spinner, meter and
 * silence countdown animate). `render()` returns the panel's lines; it's
 * empty once the utterance collapses.
 */
export class VoicePanel {
  private phase: "idle" | "warming" | "listening" | "silence" | "transcribing" = "idle";
  private level = 0;
  private partial = "";
  private silenceStartedAt = 0;

  constructor(
    private readonly silenceMs = 600,
    /** Max width for the live-transcript line; the tail is kept if longer. */
    private readonly width = 60,
  ) {}

  /** Whether the panel has anything to show (redraw loop can stop otherwise). */
  get active(): boolean {
    return this.phase !== "idle";
  }

  handle(event: VoiceEvent): void {
    switch (event.type) {
      case "warming":
        this.reset("warming");
        break;
      case "ready":
        // Only the first-boot warmup shows; later presses go straight to
        // "listening" on the STATUS event below.
        if (this.phase === "warming") this.phase = "idle";
        break;
      case "listening":
        this.reset("listening");
        break;
      case "level":
        this.level = event.rms;
        break;
      case "partial":
        this.partial = event.text;
        break;
      case "silence":
        this.phase = "silence";
        this.silenceStartedAt = Date.now();
        break;
      case "transcribing":
        this.phase = "transcribing";
        break;
      case "final":
        // Keep the text visible for the brief moment before DONE collapses.
        this.partial = event.text;
        break;
      case "done":
      case "error":
        this.reset("idle");
        break;
    }
  }

  /** The panel as an array of lines (top to bottom). Empty when idle. */
  render(): string[] {
    switch (this.phase) {
      case "warming":
        return [`${spin()} Warming up voice model…`];

      case "listening":
      case "silence": {
        const head =
          this.phase === "silence"
            ? `🔴 Listening  ${meter(this.level)}  ${countdown(this.silenceMs, this.silenceStartedAt)}`
            : `🔴 Listening  ${meter(this.level)}`;
        const transcript = this.partial ? tail(this.partial, this.width) : dim("(speak…)");
        return [head, `  ${transcript}`, dim("  esc cancel")];
      }

      case "transcribing":
        return [`${spin()} Transcribing…`, `  ${tail(this.partial, this.width)}`];

      default:
        return [];
    }
  }

  private reset(phase: VoicePanel["phase"]): void {
    this.phase = phase;
    this.level = 0;
    this.partial = "";
    this.silenceStartedAt = 0;
  }
}

function spin(): string {
  return SPINNER[Math.floor(Date.now() / 80) % SPINNER.length];
}

/** A fixed-width bar meter, filled proportionally to `rms`. */
function meter(rms: number): string {
  const filled = Math.round(
    Math.min(1, Math.max(0, rms / METER_FULL_SCALE_RMS)) * METER_CELLS,
  );
  return "▕" + "█".repeat(filled) + "·".repeat(METER_CELLS - filled) + "▏";
}

/** Shrinking silence countdown, e.g. "⏳ 0.4s". */
function countdown(silenceMs: number, startedAt: number): string {
  const remaining = Math.max(0, silenceMs - (Date.now() - startedAt));
  return `⏳ ${(remaining / 1000).toFixed(1)}s`;
}

/** Keep the last `width` chars (most recent words), prefixing "…" if cut. */
function tail(text: string, width: number): string {
  if (text.length <= width) return text;
  return "…" + text.slice(text.length - width + 1);
}

/** Dim ANSI wrapper for hint text; swap for your TUI's own style helper. */
function dim(s: string): string {
  return `\x1b[2m${s}\x1b[22m`;
}
