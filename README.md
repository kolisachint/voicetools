# voicetools

Local, offline **voice-to-text for the terminal**. A small Rust binary that
captures your microphone, stops automatically when you go quiet, and streams
recognized text on stdout using a simple line protocol — designed to drop into
a TUI (e.g. hoocode) as a push-to-talk input.

- **Parakeet-TDT** (NVIDIA) via ONNX Runtime — fast, multilingual, int8.
- **Whisper** (whisper.cpp) as an optional fallback backend.
- Models are pre-exported and downloaded on first run — no Python/NeMo needed.
- ~Single static binary; models live in your user data dir.

> Status: v0.1. The mic → VAD → resample pipeline, the setup/download wizard,
> the CLI, and the stdout protocol are implemented and unit-tested. The
> Parakeet ONNX inference path is implemented against the
> istupakov/PalatineVision export and compiles against `ort` 2.0, but the
> end-to-end decode has **not** been validated against real model weights in
> CI (models are large and gated). Validate locally with `--wav` (see below)
> and please report I/O-name mismatches.

## Install

Grab a binary from [Releases](../../releases), or build from source:

```bash
# Linux needs ALSA headers for mic capture
sudo apt-get install -y libasound2-dev   # Debian/Ubuntu

cargo build --release
# binary at target/release/voicetools
```

## First run: download a model

```bash
voicetools setup --model parakeet-v3     # recommended, ~650MB, 25 languages
# or
voicetools setup --model parakeet-v2     # English-only, ~631MB
voicetools setup --model whisper-small   # fallback (needs --features whisper)

voicetools models                        # list what's installed
```

Models are stored under your platform data directory, e.g.
`~/.local/share/voicetools/models/` on Linux.

## Transcribe

```bash
# From the mic — records until ~600ms of silence, then transcribes:
voicetools transcribe

# Tune the trailing-silence auto-stop:
voicetools transcribe --silence-ms 800

# From a WAV file (any rate/channels; great for validating a model):
voicetools transcribe --wav recording.wav
```

Environment variables: `VOICETOOLS_MODEL`, `VOICE_SILENCE_MS`.

## Serve (persistent daemon)

`transcribe` reloads the ONNX models on every invocation, which is fine for a
single call but adds a cold start to every push-to-talk press. `serve` loads
the models once and then answers commands on stdin, so repeated captures are
instant:

```bash
voicetools serve
# stdin:  START            begin mic capture + VAD
#         CANCEL           stop the current capture without transcribing
#         SHUTDOWN         exit gracefully
```

Same `--model`/`--silence-ms` flags and env vars as `transcribe`. One capture
runs at a time; the mic opens on `START` and closes when it stops.

## Output protocol

Every line on **stdout** is one event; **stderr** carries human logs and
download progress, so a consumer can parse stdout cleanly:

```text
STATUS recording        # state transitions
SEGMENT hello            # a chunk of recognized text (usually one word)
SEGMENT world
DONE                     # finished
ERROR <message>          # fatal; process exits non-zero
```

`serve` speaks the same protocol plus three daemon-only lines:

```text
READY                    # models finished loading; ready for START
LEVEL 0.0123              # live RMS energy per audio chunk, while listening
PHASE silence             # trailing silence just started, while listening
```

## Architecture

```
src/
├── main.rs            CLI + subcommands, mic→VAD→transcribe pipeline
├── mic.rs             cpal capture (native-rate mono chunks over a channel)
├── audio.rs           mono downmix, linear resample → 16 kHz, WAV loading
├── vad.rs             energy VAD with auto-stop on trailing silence
├── setup.rs           model registry + HuggingFace download wizard
├── protocol.rs        the stdout line protocol
├── serve.rs           persistent daemon: START/CANCEL/SHUTDOWN over stdin
└── transcribe/
    ├── mod.rs         Transcriber trait + backend selection
    ├── parakeet.rs    ONNX Runtime: nemo128 → encoder → decoder_joint (TDT)
    └── whisper.rs     whisper.cpp fallback (feature = "whisper")
```

Audio is captured at the device's native rate, VAD runs on those chunks (RMS
is rate-independent), and the whole buffer is resampled to 16 kHz once before
inference — avoiding per-callback resampling artifacts.

### Parakeet decoding

`nemo128.onnx` produces a 128-bin mel spectrogram, `encoder.int8.onnx` encodes
it, and the combined `decoder_joint.int8.onnx` runs greedy Token-and-Duration
Transducer (TDT) decoding. The token/duration logit split is derived from the
joint output width at runtime (`width − NUM_DURATIONS`), so the same code works
for v2 and v3 without hardcoded vocab sizes.

The ONNX input/output tensor names are `const`s at the top of
`src/transcribe/parakeet.rs`. They match the istupakov/PalatineVision export;
if you use a differently-named export, inspect it with
[Netron](https://netron.app) and adjust those constants — the decode logic
itself is export-agnostic. Decode flow is adapted from
[`jason-ni/parakeet-rs`](https://github.com/jason-ni/parakeet-rs).

## Cargo features

| Feature    | Default | Pulls in            | Backend           |
|------------|:-------:|---------------------|-------------------|
| `parakeet` |   ✓     | `ort`, `ndarray`    | Parakeet via ONNX |
| `whisper`  |         | `whisper-rs`        | whisper.cpp       |

```bash
cargo build --release                      # parakeet (default)
cargo build --release --features whisper   # add whisper fallback
```

## Releases

CI (`cargo fmt`/`clippy`/`test`) and release automation live in
[`.github/workflows/`](.github/workflows/) (`ci.yml`, `release.yml`); see
[`ci/README.md`](ci/README.md) for the workflow reference. Releases are cut via
the `/pr <patch|minor|major>` slash command: merging the labeled PR bumps the
version, tags, publishes to crates.io, and attaches macOS/Linux/Windows
binaries. Requires the `CRATES_IO_TOKEN` secret.

## Editor / TUI integration

Reference files for wiring `voicetools` into the hoocode TUI as a `ctrl+r`
push-to-talk input live in [`docs/hoocode-integration/`](docs/hoocode-integration/).

## License

MIT
