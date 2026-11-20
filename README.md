# wl-whisper

Simple dictation tool for Wayland using NVIDIA Parakeet (via [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx))

## Dependencies

wtype: Wayland type control

## Usage

1. Download a streaming Parakeet model from the [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models)

```bash
# Parakeet Unified EN 0.6B, streaming with 1120ms chunks (English, with punctuation and casing)
wget https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-streaming-1120ms.tar.bz2
tar xjf sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-streaming-1120ms.tar.bz2
```

Text is typed while you speak. Bigger chunks are more accurate but add latency. The 560ms and 240ms variants are lower latency but much heavier: on a Ryzen AI 9 HX 370, the 560ms model can't keep up with real time. For speed over accuracy, try `sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-560ms-int8-2026-06-11`.

2. Compile and run the program

```
cargo r -r -- --model sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-streaming-1120ms --key KEY_RIGHTSHIFT
```

`--model` is the directory containing `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`. Use `--threads` to set the number of CPU threads (default 4).

Use a modifier key like `KEY_RIGHTSHIFT` (the default) as the trigger. Text is typed while the key is held, and a non-modifier key would autorepeat into the focused window.

For a list of supported key names, see [docs.rs](https://docs.rs/evdev/0.12.2/evdev/struct.Key.html).

3. Hold the key and speak. Words are typed as they are recognized; release to finish.
