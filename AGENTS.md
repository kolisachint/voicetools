# Development Rules

## Repo Map

`voicetools` is a single-crate Cargo binary: a local, offline voice-to-text CLI
that captures the mic, runs VAD auto-stop, and streams recognized text on stdout.

Docs (keep these current when behavior changes):

- `README.md` — features, install, usage, output protocol, architecture
- `ci/README.md` — CI/release workflow reference
- `docs/hoocode-integration/` — TUI push-to-talk wiring reference

Code:

- `src/main.rs` — CLI + subcommands (`setup`, `models`, `transcribe`,
  `serve`), the mic → VAD → resample → transcribe pipeline
- `src/mic.rs` — `cpal` capture (native-rate mono chunks over a channel)
- `src/audio.rs` — mono downmix, anti-aliased resample → 16 kHz (windowed-sinc
  low-pass before decimation), WAV loading
- `src/vad.rs` — energy VAD with auto-stop on trailing silence
  (`Speech`/`Silence`/`SilenceStart`/`SilenceTimeout`)
- `src/setup.rs` — model registry + HuggingFace download wizard
- `src/protocol.rs` — the stdout line protocol
  (`STATUS`/`SEGMENT`/`DONE`/`ERROR`, plus `READY`/`LEVEL`/`PARTIAL`/`PHASE`/
  `FINAL` for `serve`)
- `src/serve.rs` — persistent daemon: loads models once, then answers
  `START`/`CANCEL`/`SHUTDOWN` on stdin (one capture at a time). Streams live
  `PARTIAL`s by re-decoding the audio-so-far on a `--partial-ms` interval,
  then emits one `FINAL`
- `src/transcribe/mod.rs` — `Transcriber` trait + backend selection
- `src/transcribe/parakeet.rs` — ONNX Runtime TDT decode (default feature)
- `src/transcribe/whisper.rs` — whisper.cpp fallback (feature = "whisper")
- `.github/workflows/` — `ci.yml`, `release.yml`
- `.agents/commands/` — slash-command definitions (`pr.md`)

**Cargo features**: `parakeet` (default; `ort` + `ndarray`), `whisper`
(`whisper-rs`). Mic/audio/CLI/download deps are always on.

## Conversational Style

- Keep answers short and concise
- No emojis in commits, issues, PR comments, or code
- No fluff or cheerful filler text
- Technical prose only, be kind but direct

## Code Quality

- Read files in full before making wide-ranging changes, before editing files
  you have not already fully inspected, and when asked to investigate or audit.
  Do not rely only on search snippets for broad changes.
- Match the surrounding style: import order, naming, error handling (`anyhow`
  with `?`)
- Avoid `unwrap()`/`expect()` outside tests; thread errors with `?`
- Keep the ONNX input/output tensor names as the `const`s at the top of
  `src/transcribe/parakeet.rs`; the decode logic stays export-agnostic
- Do not preserve backward compatibility unless the user explicitly asks
- Always ask before removing functionality that appears intentional

## Commands

- After code changes (not doc-only changes), run all three and fix everything
  before committing:
  ```bash
  cargo fmt --all --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```
- Linux needs ALSA headers for mic capture: `sudo apt-get install -y libasound2-dev`
- Validate a model end-to-end locally with `--wav`:
  `cargo run --release -- transcribe --wav recording.wav`
- If you create or modify a test, run it and iterate until it passes
- NEVER commit unless the user asks

## Slash Commands

- `/pr [patch|minor|major]` — opens a release PR on a feature branch and labels
  it `cargo:<bump>` so `release.yml` bumps the version, publishes to crates.io,
  and builds cross-platform binaries on merge. Defined in `.agents/commands/pr.md`.
  Defaults to `patch`.
- Slash-command definitions live in `.agents/commands/`.

## Releasing

**Version semantics**:

- `patch` — bug fixes and additions
- `minor` — API/behavior changes
- `major` — large breaking changes

### Flow (do NOT bump versions or tag by hand)

**Never edit `version = "…"` in `Cargo.toml` inside a feature PR.** The release
workflow is the sole owner of the version: it computes the next version from the
latest `v*` git tag plus the PR's `cargo:<bump>` label, then rewrites the
manifest. A manual bump is at best ignored and at worst confusing (it can cause
a skipped version number). Leave the version untouched and just apply the label.

1. `/pr <bump>` opens a PR labeled `cargo:<bump>`.
2. On merge, `release.yml` derives the next version from the latest `v*` tag,
   bumps `Cargo.toml`, updates `Cargo.lock`, commits `release: v<version>`,
   tags `v<version>`, and pushes `main`.
3. The same workflow then publishes `voicetools` to crates.io (skipping a
   version already on the index) and builds + attaches binaries for macOS
   (Apple silicon + Intel), Linux (`x86_64-unknown-linux-gnu`), and Windows to
   the GitHub release.

Secrets required: `CRATES_IO_TOKEN` (crates.io publish). `GITHUB_TOKEN` is
provided automatically.

Manual fallback (only if asked): `git tag vX.Y.Z && git push origin vX.Y.Z`.

## **CRITICAL** Git Rules for Parallel Agents **CRITICAL**

Multiple agents may work on different files in the same worktree simultaneously.

### Committing

- ONLY commit files YOU changed in THIS session
- Include `fixes #<number>` / `closes #<number>` when there is a related issue/PR
- NEVER use `git add -A` or `git add .` — these sweep up other agents' changes
- ALWAYS `git add <specific-file-paths>` listing only files you modified
- Run `git status` before committing and verify you are staging only YOUR files

### Forbidden Git Operations

These can destroy other agents' work and are never allowed:

- `git reset --hard`
- `git checkout .`
- `git clean -fd`
- `git stash`
- `git add -A` / `git add .`
- `git commit --no-verify`

### Safe Workflow

```bash
git status                       # 1. check first
git add src/vad.rs               # 2. stage only your files
git commit -m "fix(vad): ..."    # 3. commit
git pull --rebase && git push    # 4. push (never reset/checkout)
```

### If Rebase Conflicts Occur

- Resolve conflicts in YOUR files only
- If a conflict is in a file you did not modify, abort and ask the user
- NEVER force push over shared history

### User Override

If the user's instructions conflict with these rules, ask for confirmation that
they want to override. Only then proceed.
