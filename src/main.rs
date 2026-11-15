use anyhow::{Context, Result, anyhow};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use evdev::{Device, InputEventKind, Key};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

const SAMPLE_RATE: u32 = 16_000;

#[derive(Parser)]
#[command(about = "Hold key to record, release to transcribe and insert text")]
struct Args {
    /// Directory with the Parakeet ONNX files (encoder, decoder, joiner, tokens.txt)
    #[arg(short, long)]
    model: PathBuf,
    /// Number of CPU threads used for inference
    #[arg(short, long, default_value = "4")]
    threads: i32,
    #[arg(short, long, default_value = "KEY_RIGHTSHIFT")]
    key: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    eprintln!("Loading model: {}", args.model.display());
    let recognizer = load_model(&args.model, args.threads)?;
    eprintln!("Model ready.");

    let key_trigger = Key::from_str(&args.key)
        .map_err(|_| anyhow!("Invalid key {}! For a list of supported key names, see https://docs.rs/evdev/0.12.2/evdev/struct.Key.html", args.key))?;

    let keyboards = find_keyboards(key_trigger);
    if keyboards.is_empty() {
        return Err(anyhow!(
            "No device found that supports {}. Check /dev/input permissions or try a different key.",
            args.key
        ));
    }

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
        // Skip too-short samples
        let duration = raw.len() as f32 / sample_rate as f32;
        if duration < 0.5 {
            eprintln!("[Audio too short! {:.1}s]", duration);
            continue;
        }

        eprintln!("[Transcribing {:.1}s]", duration);
        let audio = if sample_rate != SAMPLE_RATE {
            resample(&raw, sample_rate, SAMPLE_RATE)?
        } else {
            raw
        };

        match transcribe(&recognizer, &audio) {
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

fn find_keyboards(trigger: Key) -> Vec<Device> {
    evdev::enumerate()
        .filter_map(|(_, dev)| {
            dev.supported_keys()
                .map_or(false, |k| k.contains(trigger))
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

fn load_model(dir: &Path, threads: i32) -> Result<OfflineRecognizer> {
    let mut config = OfflineRecognizerConfig::default();
    let model = &mut config.model_config;
    model.transducer.encoder = Some(find_file(dir, "encoder", ".onnx")?);
    model.transducer.decoder = Some(find_file(dir, "decoder", ".onnx")?);
    model.transducer.joiner = Some(find_file(dir, "joiner", ".onnx")?);
    model.tokens = Some(find_file(dir, "tokens", ".txt")?);
    model.model_type = Some("nemo_transducer".into());
    model.num_threads = threads;
    model.provider = Some("cpu".into());
    config.decoding_method = Some("greedy_search".into());
    OfflineRecognizer::create(&config).context("Failed to load model")
}

/// Find a file in `dir` named `<prefix>*<suffix>`, e.g. `encoder.int8.onnx`.
fn find_file(dir: &Path, prefix: &str, suffix: &str) -> Result<String> {
    let mut matches: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("Cannot read model directory {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(prefix) && n.ends_with(suffix)
            })
        })
        .collect();
    matches.sort();
    match matches.as_slice() {
        [one] => Ok(one.to_string_lossy().into_owned()),
        [] => Err(anyhow!("No {prefix}*{suffix} in {}", dir.display())),
        _ => Err(anyhow!(
            "Multiple {prefix}*{suffix} files in {}: {matches:?}",
            dir.display()
        )),
    }
}

fn transcribe(recognizer: &OfflineRecognizer, audio: &[f32]) -> Result<String> {
    let stream = recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, audio);
    recognizer.decode(&stream);
    let result = stream.get_result().context("Inference failed")?;
    Ok(result.text.trim().to_string())
}
