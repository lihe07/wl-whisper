use anyhow::{Context, Result, anyhow};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use evdev::{Device, InputEventKind, Key};
use std::str::FromStr;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const WHISPER_SAMPLE_RATE: u32 = 16_000;

#[derive(Parser)]
#[command(about = "Hold key to record, release to transcribe and insert text")]
struct Args {
    #[arg(short, long)]
    model: String,
    #[arg(short, long, default_value = "auto")]
    language: String,
    #[arg(long, default_value = "0")]
    gpu: i32,
    #[arg(short, long, default_value = "KEY_RIGHTSHIFT")]
    key: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    eprintln!("Loading model: {}", args.model);
    let mut ctx_params = WhisperContextParameters::default();
    ctx_params.use_gpu(true);
    ctx_params.gpu_device = args.gpu;
    let ctx =
        WhisperContext::new_with_params(&args.model, ctx_params).context("Failed to load model")?;
    eprintln!("Model ready.");

    let keyboards = find_keyboards();
    if keyboards.is_empty() {
        return Err(anyhow!(
            "No keyboard found in /dev/input. Are you in the 'input' group?\n  sudo usermod -aG input $USER"
        ));
    }

    let key_trigger = Key::from_str(&args.key)
        .map_err(|_| anyhow!("Invalid key {}! For a list of supported key names, see https://docs.rs/evdev/0.12.2/evdev/struct.Key.html", args.key))?;

    let (key_tx, key_rx) = mpsc::channel::<bool>();
    for mut kb in keyboards {
        let tx = key_tx.clone();
        thread::spawn(move || {
            loop {
                match kb.fetch_events() {
                    Ok(events) => {
                        for ev in events {
                            if let InputEventKind::Key(key) = ev.kind() {
                                if key == key_trigger {
                                    match ev.value() {
                                        1 => {
                                            let _ = tx.send(true);
                                        }
                                        0 => {
                                            let _ = tx.send(false);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Keyboard error: {e}");
                        break;
                    }
                }
            }
        });
    }
    drop(key_tx);

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("No audio input device")?;
    let supported = device.default_input_config()?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let stream_config: cpal::StreamConfig = supported.into();

    eprintln!(
        "Mic: {} | {}Hz",
        device.name().unwrap_or_else(|_| "?".into()),
        sample_rate
    );
    eprintln!("Hold {} to record. Ctrl+C to quit.\n", args.key);

    loop {
        match key_rx.recv() {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => break,
        }

        let notif_id = show_notification();
        eprintln!("[Recording…]");

        let raw_buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let stream = build_input_stream(
            &device,
            &stream_config,
            sample_format,
            Arc::clone(&raw_buf),
            channels,
        )?;
        stream.play()?;

        loop {
            match key_rx.recv() {
                Ok(false) => break,
                Ok(true) | Err(_) => {}
            }
        }

        drop(stream);
        dismiss_notification(notif_id);

        let raw = raw_buf.lock().unwrap().clone();
        if raw.is_empty() {
            continue;
        }

        eprintln!(
            "[Transcribing {:.1}s…]",
            raw.len() as f32 / sample_rate as f32
        );
        let audio = if sample_rate != WHISPER_SAMPLE_RATE {
            resample(&raw, sample_rate, WHISPER_SAMPLE_RATE)?
        } else {
            raw
        };

        match transcribe(&ctx, &audio, &args.language) {
            Ok(text) if !text.is_empty() => {
                if let Err(e) = std::process::Command::new("wtype").arg(&text).status() {
                    eprintln!("wtype failed: {e}");
                }
                eprintln!("[Done: {text}]");
            }
            Ok(_) => eprintln!("[No speech detected]"),
            Err(e) => eprintln!("[Error: {e}]"),
        }
    }

    Ok(())
}

fn find_keyboards() -> Vec<Device> {
    evdev::enumerate()
        .filter_map(|(_, dev)| {
            dev.supported_keys()
                .map_or(false, |k| k.contains(Key::KEY_RIGHTALT))
                .then_some(dev)
        })
        .collect()
}

fn show_notification() -> Option<u32> {
    let out = std::process::Command::new("notify-send")
        .args([
            "--urgency=critical",
            "-t",
            "0",
            "-p",
            "wl-whisper",
            "● Recording…",
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn dismiss_notification(id: Option<u32>) {
    let Some(id) = id else { return };
    std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.CloseNotification",
            &id.to_string(),
        ])
        .output()
        .ok();
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    samples: Arc<Mutex<Vec<f32>>>,
    channels: usize,
) -> Result<cpal::Stream> {
    let err_fn = |e| eprintln!("Audio error: {e}");
    macro_rules! stream_for {
        ($t:ty, $to_f32:expr) => {{
            let s = samples.clone();
            device.build_input_stream(
                config,
                move |data: &[$t], _| {
                    let mut buf = s.lock().unwrap();
                    for frame in data.chunks(channels) {
                        buf.push(
                            frame.iter().map(|&x| ($to_f32)(x)).sum::<f32>() / channels as f32,
                        );
                    }
                },
                err_fn,
                None,
            )?
        }};
    }
    Ok(match format {
        cpal::SampleFormat::F32 => stream_for!(f32, |x: f32| x),
        cpal::SampleFormat::I16 => stream_for!(i16, |x: i16| x as f32 / i16::MAX as f32),
        cpal::SampleFormat::U16 => {
            stream_for!(u16, |x: u16| (x as f32 / u16::MAX as f32) * 2.0 - 1.0)
        }
        other => return Err(anyhow!("Unsupported sample format: {other:?}")),
    })
}

fn resample(input: &[f32], from_hz: u32, to_hz: u32) -> Result<Vec<f32>> {
    use rubato::{FftFixedIn, Resampler};
    const CHUNK: usize = 1024;
    let mut r = FftFixedIn::<f32>::new(from_hz as usize, to_hz as usize, CHUNK, 2, 1)
        .map_err(|e| anyhow!("{e:?}"))?;
    let mut out =
        Vec::with_capacity((input.len() as f64 * to_hz as f64 / from_hz as f64) as usize + CHUNK);
    let mut pos = 0;
    while pos < input.len() {
        let end = (pos + CHUNK).min(input.len());
        let mut buf = vec![0.0f32; CHUNK];
        buf[..end - pos].copy_from_slice(&input[pos..end]);
        out.extend_from_slice(&r.process(&[buf], None).map_err(|e| anyhow!("{e:?}"))?[0]);
        pos += CHUNK;
    }
    Ok(out)
}

fn transcribe(ctx: &WhisperContext, audio: &[f32], language: &str) -> Result<String> {
    let mut state = ctx.create_state().context("Failed to create state")?;
    let mut params = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: 5,
        patience: 1.0,
    });
    params.set_language(if language == "auto" {
        None
    } else {
        Some(language)
    });
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    state.full(params, audio).context("Inference failed")?;

    let mut result = String::new();
    for i in 0..state.full_n_segments() {
        if let Some(seg) = state.get_segment(i) {
            if let Ok(text) = seg.to_str() {
                let text = text.trim();
                if !text.is_empty() {
                    if !result.is_empty() {
                        result.push(' ');
                    }
                    result.push_str(text);
                }
            }
        }
    }
    Ok(result)
}
