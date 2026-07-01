# Mirrored model files

The `models-v1` GitHub release republishes unmodified copies of the model
files `voicetools setup` downloads, as a fallback for networks that can't
reach huggingface.co (e.g. behind a corporate proxy). `voicetools setup`
tries huggingface.co first and only falls back to this mirror on failure.

This is a one-time mirror (see `ci-templates/mirror-models.yml`, not yet
installed as an active workflow — see TODO below), not kept continuously in
sync with upstream.

## Sources and licenses

- **parakeet-v3** — [PalatineVision/parakeet-tdt-0.6b-v3-onnx](https://huggingface.co/PalatineVision/parakeet-tdt-0.6b-v3-onnx),
  an ONNX export of NVIDIA's Parakeet-TDT weights. License: CC-BY-4.0.
- **parakeet-v2** — [istupakov/parakeet-tdt-0.6b-v2-onnx](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx),
  an ONNX export of NVIDIA's Parakeet-TDT weights. License: CC-BY-4.0.
- **whisper-small** — [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp)
  `ggml-small.en.bin`, a ggml conversion of OpenAI's Whisper small.en weights. License: MIT.

Files are redistributed unmodified, with attribution to the original
repositories above, per each license's terms.

## TODO: finish setting up the mirror

The workflow that populates `models-v1` is ready but sits at
`ci-templates/mirror-models.yml` instead of `.github/workflows/`, because
the session that authored it could only push via an OAuth token without the
`workflow` scope GitHub requires for changes under `.github/workflows/`.
Until it's installed there and run once, `setup`'s fallback path will always
404 (harmless — huggingface.co is still tried first).

Steps to finish, from a machine with normal `git`/GitHub push access:

1. `git mv ci-templates/mirror-models.yml .github/workflows/mirror-models.yml`,
   commit, and push. (Or delete `ci-templates/` and add the same content
   directly under `.github/workflows/`.)
2. In the GitHub UI: Actions → "Mirror models" → Run workflow. This
   downloads the model files from huggingface.co and publishes them as
   assets on a new `models-v1` release.
3. Confirm the release exists with all 9 assets (4 files each for
   parakeet-v3/parakeet-v2, 1 for whisper-small).
4. Test the fallback: `voicetools setup --model whisper-small` should
   still work if you temporarily block huggingface.co, by falling through
   to `models-v1`.
5. Once confirmed, delete `ci-templates/mirror-models.yml` (no longer
   needed — the real workflow lives in `.github/workflows/`).

No further code changes are needed — `setup.rs` already points at
`models-v1` release assets.
