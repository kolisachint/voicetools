# hoocode integration

These files wire `voicetools` into the hoocode TUI as a `ctrl+r` push-to-talk
voice input. They live here (rather than in the hoocode repo) so the voicetools
release carries its own integration recipe; copy/apply them in hoocode.

## 1. New file: `packages/tui/src/voice-transcribe.ts`

Copy [`voice-transcribe.ts`](./voice-transcribe.ts) into `packages/tui/src/`.
It spawns `voicetools transcribe`, parses the stdout line protocol, and injects
recognized text via bracketed paste.

## 2. `packages/tui/src/keybindings.ts` — add one binding

```ts
// In `interface Keybindings`:
"tui.input.voiceTranscribe": true;

// In `TUI_KEYBINDINGS`:
"tui.input.voiceTranscribe": { defaultKeys: "ctrl+r", description: "Voice input" },
```

## 3. `packages/tui/src/tui.ts` — handle the keypress

```ts
import { transcribeToInput } from "./voice-transcribe.js";
import { resolveVoicetoolsBin } from "@hoocode/coding-agent/config";

// Inside the keypress handler:
if (keybindings.matches(data, "tui.input.voiceTranscribe")) {
  const statusMap: Record<string, string> = {
    recording: "🎙  Speak now…",
    transcribing: "✦  Transcribing…",
    ready: "✓  Ready",
  };
  transcribeToInput(
    resolveVoicetoolsBin(),
    stdinBuffer,
    (s) =>
      ctx.ui.notify(
        statusMap[s] ?? s,
        s.startsWith("error") ? "error" : "info",
      ),
  );
  return;
}
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

- The TUI never parses stderr — only the stdout protocol
  (`STATUS` / `SEGMENT` / `DONE` / `ERROR`). Keep that contract intact.
- `VOICETOOLS_BIN` lets users point at a specific binary; otherwise a bundled
  `bin/voicetools` is preferred, then `PATH`.
