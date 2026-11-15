# wl-whisper

Simple dictation tool for Wayland using NVIDIA Parakeet (via [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx))

## Dependencies

wtype: Wayland type control

## Usage

1. Download a Parakeet model from the [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models)

```bash
# English only. For 25 European languages, use sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8 instead.
wget https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2
tar xjf sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2
```

2. Compile and run the program

```
cargo r -r -- --model sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 --key KEY_RIGHTSHIFT
```

`--model` is the directory containing `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`. Use `--threads` to set the number of CPU threads (default 4).

For a list of supported key names, see [docs.rs](https://docs.rs/evdev/0.12.2/evdev/struct.Key.html).

3. Press to dictate
