# Mirrored model files

The `models-v1` GitHub release republishes unmodified copies of the model
files `voicetools setup` downloads, as a fallback for networks that can't
reach huggingface.co (e.g. behind a corporate proxy). `voicetools setup`
tries huggingface.co first and only falls back to this mirror on failure.

This is a one-time mirror (see `.github/workflows/mirror-models.yml`), not
kept continuously in sync with upstream.

## Sources and licenses

- **parakeet-v3** — [PalatineVision/parakeet-tdt-0.6b-v3-onnx](https://huggingface.co/PalatineVision/parakeet-tdt-0.6b-v3-onnx),
  an ONNX export of NVIDIA's Parakeet-TDT weights. License: CC-BY-4.0.
- **parakeet-v2** — [istupakov/parakeet-tdt-0.6b-v2-onnx](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx),
  an ONNX export of NVIDIA's Parakeet-TDT weights. License: CC-BY-4.0.
- **whisper-small** — [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp)
  `ggml-small.en.bin`, a ggml conversion of OpenAI's Whisper small.en weights. License: MIT.

Files are redistributed unmodified, with attribution to the original
repositories above, per each license's terms.
