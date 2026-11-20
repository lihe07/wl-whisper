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

Other offline models with the same layout also work:

- `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`: 25 European languages
- `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`: older English model

Streaming models are not supported. They are less accurate and can't revise words once they're typed.

2. Compile and run the program

```
cargo r -r -- --model sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming --key KEY_RIGHTSHIFT
```

`--model` is the directory containing `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`. Use `--threads` to set the number of CPU threads (default 4).

Key presses shorter than 0.5s are ignored, so the trigger key keeps working normally when tapped. If you use keyd, add `rightshift = rightshift` to its `[main]` section, otherwise keyd reports Right Shift as Left Shift.

For a list of supported key names, see [docs.rs](https://docs.rs/evdev/0.12.2/evdev/struct.Key.html).

3. Hold the key and speak, then release to insert the text.
