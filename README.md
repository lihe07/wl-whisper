# wl-whisper

Simple dictation tool for Wayland using OpenAI Whisper 

## Dependencies

wtype: Wayland type control

hipblas or CUDA (optional): GPU inference

## Usage

1. Download a Whisper model from [GGML HuggingFace](https://huggingface.co/ggerganov/whisper.cpp/tree/main)

```bash
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin
```

2. Compile and run the program

```
cargo r -r --model <path to ggml.bin>
```

3. Press Right Alt to dictate
