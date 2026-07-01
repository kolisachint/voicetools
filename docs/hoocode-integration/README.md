# hoocode integration

These files wire `voicetools` into the hoocode TUI as a `ctrl+r` push-to-talk
voice input. They live here (rather than in the hoocode repo) so the voicetools
release carries its own integration recipe; copy/apply them in hoocode.

`voicetools serve` keeps models loaded in a background daemon for the life of
the TUI process, so push-to-talk has no per-press cold start (the first press
still waits for `READY`; every press after that jumps straight to
listening). Crucially it transcribes **live**: it streams `PARTIAL` lines as
you speak and commits one `FINAL` at the end, so text builds up in the panel
in real time instead of appearing all at once. Binaries built before `serve`
existed are detected and fall back to the previous spawn-per-press behavior
automatically — no hoocode-side version gate needed.

## 1. New file: `packages/tui/src/voice-transcribe.ts`

Copy [`voice-transcribe.ts`](./voice-transcribe.ts) into `packages/tui/src/`.
It exports:

- `VoiceDaemon` — spawns `voicetools serve` once, keeps it alive across
  presses, respawns it if it crashes, and writes `START`/`CANCEL` to its
  stdin. Probes support with `voicetools serve --help` (exits `0` iff the
  subcommand exists, regardless of model-load time) and falls back to
  `transcribeToInput` (spawn-per-press) for binaries that predate `serve`.
  Only `FINAL` is injected into the input (via bracketed paste); `PARTIAL`s
  are shown but never committed.
- `VoicePanel` — turns the `VoiceEvent` stream into a **multi-line live
  panel** (`render()` returns an array of lines): a spinner while warming up;
  a `🔴 Listening` header with a live level meter; the transcript building up
  word-by-word from `PARTIAL`s on the line below; a shrinking silence
  countdown once trailing silence starts; a dim `esc cancel` footer. It
  collapses to `[]` on `DONE`/`ERROR`.

## 2. `packages/tui/src/keybindings.ts` — add one binding

```ts
// In `interface Keybindings`:
"tui.input.voiceTranscribe": true;

// In `TUI_KEYBINDINGS`:
"tui.input.voiceTranscribe": { defaultKeys: "ctrl+r", description: "Voice input" },
```

## 3. `packages/tui/src/tui.ts` — wire the daemon and render the panel

```ts
import { VoiceDaemon, VoicePanel } from "./voice-transcribe.js";
import { resolveVoicetoolsBin } from "@hoocode/coding-agent/config";

// Created once for the TUI process lifetime (not per keypress).
const voicePanel = new VoicePanel();
const voiceDaemon = new VoiceDaemon(resolveVoicetoolsBin(), stdinBuffer, (event) => {
  voicePanel.handle(event);
  if (event.type === "error") ctx.ui.notify(event.message, "error");
  ctx.ui.setVoicePanel(voicePanel.render()); // string[]; [] once collapsed
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

`VoicePanel.render()` returns the panel as an array of lines; draw it as a
small region just above the input (or wherever transient status lives). While
`voicePanel.active` is true, redraw on a short interval (~80ms) so the
spinner, level meter and silence countdown animate — re-rendering only the
panel region is enough, no full repaint. When it returns `[]`, clear the
region.

The `esc cancel` hint lives inside the panel's own dim footer line, so there's
nothing extra to add to the global key-hints row.

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

- The TUI never parses stderr — only the stdout protocol. `serve` speaks
  `READY` / `STATUS` / `LEVEL` / `PARTIAL` / `PHASE` / `FINAL` / `DONE` /
  `ERROR`; the pre-`serve` fallback path still speaks `STATUS` / `SEGMENT` /
  `DONE` / `ERROR`. Keep that contract intact.
- `VOICETOOLS_BIN` lets users point at a specific binary; otherwise a bundled
  `bin/voicetools` is preferred, then `PATH`.
- `VoiceDaemon` sends `SHUTDOWN` and gives the process a short grace period on
  `dispose()` before force-killing it — call `dispose()` once, on TUI exit,
  not per press.
