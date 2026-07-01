# hoocode integration

These files wire `voicetools` into the hoocode TUI as a `ctrl+r` push-to-talk
voice input. They live here (rather than in the hoocode repo) so the voicetools
release carries its own integration recipe; copy/apply them in hoocode.

`voicetools serve` keeps models loaded in a background daemon for the life of
the TUI process, so push-to-talk has no per-press cold start (the first press
still waits for `READY`; every press after that jumps straight to
listening). Binaries built before `serve` existed are detected and fall back
to the previous spawn-per-press behavior automatically — no hoocode-side
version gate needed.

## 1. New file: `packages/tui/src/voice-transcribe.ts`

Copy [`voice-transcribe.ts`](./voice-transcribe.ts) into `packages/tui/src/`.
It exports:

- `VoiceDaemon` — spawns `voicetools serve` once, keeps it alive across
  presses, respawns it if it crashes, and writes `START`/`CANCEL` to its
  stdin. Probes support with `voicetools serve --help` (exits `0` iff the
  subcommand exists, regardless of model-load time) and falls back to
  `transcribeToInput` (spawn-per-press) for binaries that predate `serve`.
- `VoiceStatusLine` — turns the `VoiceEvent` stream into one compact status
  string: a spinner while warming up, `🔴 Listening` plus a live level meter
  while capturing, a shrinking silence countdown once trailing silence
  starts, and it collapses back to `""` on `DONE`/`ERROR`.

## 2. `packages/tui/src/keybindings.ts` — add one binding

```ts
// In `interface Keybindings`:
"tui.input.voiceTranscribe": true;

// In `TUI_KEYBINDINGS`:
"tui.input.voiceTranscribe": { defaultKeys: "ctrl+r", description: "Voice input" },
```

## 3. `packages/tui/src/tui.ts` — wire the daemon and keypress

```ts
import { VoiceDaemon, VoiceStatusLine } from "./voice-transcribe.js";
import { resolveVoicetoolsBin } from "@hoocode/coding-agent/config";

// Created once for the TUI process lifetime (not per keypress).
const voiceStatus = new VoiceStatusLine();
const voiceDaemon = new VoiceDaemon(resolveVoicetoolsBin(), stdinBuffer, (event) => {
  voiceStatus.handle(event);
  if (event.type === "error") ctx.ui.notify(event.message, "error");
  ctx.ui.setStatusLine(voiceStatus.render()); // render() is "" once collapsed
});

let voiceSession: { cancel(): void } | null = null;

// Inside the keypress handler:
if (keybindings.matches(data, "tui.input.voiceTranscribe")) {
  voiceSession = await voiceDaemon.startCapture();
  return;
}
// A second binding (e.g. `esc`) cancels the in-flight capture:
if (keybindings.matches(data, "tui.input.voiceCancel")) {
  voiceSession?.cancel();
  return;
}

// On TUI shutdown:
voiceDaemon.dispose();
```

Redraw `ctx.ui.setStatusLine(voiceStatus.render())` on an interval (e.g. every
100ms) while `voiceStatus.active` is true, so the spinner/meter/countdown
animate; a status-line-only redraw is enough, no full repaint needed.

Move the old inline cancel hint out of the status line and into the
persistent dim key footer (next to the other keybinding hints), since the
status line is now busy with the meter/countdown:

```ts
// In the footer/key-hints row:
if (voiceStatus.active) footerHints.push(dim("esc cancel"));
```

## 4. `packages/coding-agent/src/config.ts` — resolve the binary

```ts
import { existsSync } from "node:fs";
import { join } from "node:path";

export function resolveVoicetoolsBin(): string {
  if (process.env.VOICETOOLS_BIN) return process.env.VOICETOOLS_BIN;
  const bundled = join(getHooCodeDir(), "bin", "voicetools");
  if (existsSync(bundled)) return bundled;
  return "voicetools"; // fall through to PATH
}
```

## Notes

- The TUI never parses stderr — only the stdout protocol (`READY` / `STATUS`
  / `LEVEL` / `PHASE` / `SEGMENT` / `DONE` / `ERROR`). Keep that contract
  intact.
- `VOICETOOLS_BIN` lets users point at a specific binary; otherwise a bundled
  `bin/voicetools` is preferred, then `PATH`.
- `VoiceDaemon` sends `SHUTDOWN` and gives the process a short grace period on
  `dispose()` before force-killing it — call `dispose()` once, on TUI exit,
  not per press.
