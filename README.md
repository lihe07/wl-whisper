# wl-whisper

Simple dictation tool for Wayland using NVIDIA Parakeet (via [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx))

## Dependencies

wtype: Wayland type control

## Usage

1. Download a Parakeet model from the [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models)

```bash
# Recommended: Parakeet Unified EN 0.6B (English, with punctuation and casing)
wget https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming.tar.bz2
tar xjf sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming.tar.bz2
```

Other models with the same layout also work:

- `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`: 25 European languages
- `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`: older English model
- The full-precision (fp32, ~2.5 GB) Unified EN export from [Hugging Face](https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-non-streaming). It's slightly more accurate than int8 but much larger and slower on CPU.

2. Compile and run the program

```
cargo r -r -- --model sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming --key KEY_RIGHTSHIFT
```

`--model` is the directory containing `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`. Use `--threads` to set the number of CPU threads (default 4).

For a list of supported key names, see [docs.rs](https://docs.rs/evdev/0.12.2/evdev/struct.Key.html).

3. Press to dictate
