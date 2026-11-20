use anyhow::{Context, Result, anyhow};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use evdev::{Device, InputEventKind, Key};
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;

/// Audio kept from before the key press, so the first word is not clipped
const PREROLL_SECS: f32 = 0.3;
/// Silence appended on release to flush the final words out of the model
const TAIL_PADDING_SECS: f32 = 0.6;

#[derive(Parser)]
#[command(about = "Hold key to dictate; text is typed as you speak")]
struct Args {
    /// Directory with the streaming Parakeet ONNX files (encoder, decoder, joiner, tokens.txt)
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

    let (tx, rx) = mpsc::channel::<Event>();
    for mut kb in keyboards {
        let tx = tx.clone();
        thread::spawn(move || {
            loop {
                match kb.fetch_events() {
                    Ok(events) => {
                        for ev in events {
                            if ev.kind() == InputEventKind::Key(key_trigger) {
                                match ev.value() {
                                    1 => {
                                        let _ = tx.send(Event::Key(true));
                                    }
                                    0 => {
                                        let _ = tx.send(Event::Key(false));
                                    }
                                    _ => {}
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

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("No audio input device")?;
    let supported = device.default_input_config()?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let stream_config: cpal::StreamConfig = supported.into();

    // The mic stays open so recording starts instantly on key press.
    let mic = build_input_stream(&device, &stream_config, sample_format, tx, channels)?;
    mic.play()?;

    eprintln!(
        "Mic: {} | {}Hz",
        device.name().unwrap_or_else(|_| "?".into()),
        sample_rate
    );
    eprintln!("Hold {} to dictate. Ctrl+C to quit.\n", args.key);

    let preroll_len = (sample_rate as f32 * PREROLL_SECS) as usize;
    let mut preroll: VecDeque<f32> = VecDeque::with_capacity(preroll_len);
    let mut session: Option<Session> = None;

    for event in rx {
        match event {
            Event::Audio(chunk) => match &mut session {
                Some(s) => {
                    s.stream.accept_waveform(sample_rate as i32, &chunk);
                    decode_ready(&recognizer, &s.stream);
                    let text = current_text(&recognizer, &s.stream);
                    // Hold back the last word, it may still change.
                    let settled = text.rfind(' ').map_or("", |i| &text[..i]);
                    type_diff(&mut s.typed, settled);
                }
                None => {
                    preroll.extend(chunk);
                    let excess = preroll.len().saturating_sub(preroll_len);
                    preroll.drain(..excess);
                }
            },
            Event::Key(true) if session.is_none() => {
                let stream = recognizer.create_stream();
                stream.accept_waveform(sample_rate as i32, preroll.make_contiguous());
                preroll.clear();
                session = Some(Session {
                    stream,
                    typed: String::new(),
                    notif: show_notification(),
                });
            }
            Event::Key(false) => {
                let Some(mut s) = session.take() else { continue };
                dismiss_notification(s.notif);
                let tail = vec![0.0f32; (sample_rate as f32 * TAIL_PADDING_SECS) as usize];
                s.stream.accept_waveform(sample_rate as i32, &tail);
                s.stream.input_finished();
                decode_ready(&recognizer, &s.stream);
                let text = current_text(&recognizer, &s.stream);
                type_diff(&mut s.typed, &text);
                if text.is_empty() {
                    eprintln!("[No speech detected]");
                } else {
                    eprintln!("[Done: {text}]");
                }
            }
            Event::Key(true) => {}
        }
    }

    Ok(())
}

enum Event {
    Key(bool),
    Audio(Vec<f32>),
}

struct Session {
    stream: OnlineStream,
    /// Text already typed into the focused window
    typed: String,
    notif: Option<u32>,
}

fn decode_ready(recognizer: &OnlineRecognizer, stream: &OnlineStream) {
    while recognizer.is_ready(stream) {
        recognizer.decode(stream);
    }
}

fn current_text(recognizer: &OnlineRecognizer, stream: &OnlineStream) -> String {
    recognizer
        .get_result(stream)
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default()
}

/// Bring the typed text in line with `target`, backspacing over anything the model revised.
fn type_diff(typed: &mut String, target: &str) {
    let common = typed
        .chars()
        .zip(target.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let backspaces = typed.chars().count() - common;
    let addition: String = target.chars().skip(common).collect();
    if backspaces == 0 && addition.is_empty() {
        return;
    }
    let mut cmd = std::process::Command::new("wtype");
    for _ in 0..backspaces {
        cmd.args(["-k", "BackSpace"]);
    }
    if !addition.is_empty() {
        cmd.arg(&addition);
    }
    match cmd.status() {
        Ok(_) => *typed = target.to_string(),
        Err(e) => eprintln!("wtype failed: {e}"),
    }
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
    tx: mpsc::Sender<Event>,
    channels: usize,
) -> Result<cpal::Stream> {
    let err_fn = |e| eprintln!("Audio error: {e}");
    macro_rules! stream_for {
        ($t:ty, $to_f32:expr) => {{
            device.build_input_stream(
                config,
                move |data: &[$t], _| {
                    let mono = data
                        .chunks(channels)
                        .map(|frame| {
                            frame.iter().map(|&x| ($to_f32)(x)).sum::<f32>() / channels as f32
                        })
                        .collect();
                    let _ = tx.send(Event::Audio(mono));
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

fn load_model(dir: &Path, threads: i32) -> Result<OnlineRecognizer> {
    let mut config = OnlineRecognizerConfig::default();
    let model = &mut config.model_config;
    model.transducer.encoder = Some(find_file(dir, "encoder", ".onnx")?);
    model.transducer.decoder = Some(find_file(dir, "decoder", ".onnx")?);
    model.transducer.joiner = Some(find_file(dir, "joiner", ".onnx")?);
    model.tokens = Some(find_file(dir, "tokens", ".txt")?);
    model.num_threads = threads;
    model.provider = Some("cpu".into());
    config.decoding_method = Some("greedy_search".into());
    OnlineRecognizer::create(&config).context("Failed to load model")
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
