//! Microphone capture. One thread owns the cpal stream (streams are not
//! `Send` on every backend); the rest of the app talks to it through a
//! small handle. Samples are down-mixed to mono at the device rate and only
//! resampled to 16 kHz when a recording ends.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant};

pub const WHISPER_RATE: u32 = 16_000;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MicDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn list_devices() -> Vec<MicDevice> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    let mut out = vec![];
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            if let Ok(name) = d.name() {
                // Skip ALSA plumbing that is never a real microphone.
                if cfg!(target_os = "linux") && is_alsa_noise(&name) {
                    continue;
                }
                if out.iter().any(|m: &MicDevice| m.id == name) {
                    continue;
                }
                out.push(MicDevice { is_default: default.as_deref() == Some(&name), id: name.clone(), name: pretty(&name) });
            }
        }
    }
    out.sort_by_key(|m| !m.is_default);
    out
}

fn is_alsa_noise(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["null", "dmix", "dsnoop", "surround", "iec958", "front:", "rear:", "center_lfe", "side:", "lavrate", "samplerate", "speexrate", "upmix", "vdownmix", "jack", "oss"]
        .iter()
        .any(|p| n.starts_with(p))
}

fn pretty(name: &str) -> String {
    // ALSA names look like "sysdefault:CARD=Microphones"; show the card.
    if let Some(card) = name.split("CARD=").nth(1) {
        return card.split(',').next().unwrap_or(card).replace('_', " ");
    }
    name.to_string()
}

fn pick_device(id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(id) = id {
        if let Ok(mut devs) = host.input_devices() {
            if let Some(d) = devs.find(|d| d.name().map(|n| n == id).unwrap_or(false)) {
                return Ok(d);
            }
        }
        log::warn!("mic {id:?} not found, falling back to default");
    }
    host.default_input_device().ok_or_else(|| anyhow!("No microphone found"))
}

/// Shared between the capture thread and the app.
struct Shared {
    samples: Mutex<Vec<f32>>,
    /// Latest level, 0..1, as f32 bits.
    level: AtomicU32,
    /// Whether to keep samples (false = meter only).
    recording: AtomicBool,
    last_voice: Mutex<Instant>,
}

pub struct Capture {
    shared: Arc<Shared>,
    stop: mpsc::Sender<()>,
    rate: u32,
    started: Instant,
}

impl Capture {
    /// Open the microphone. `record = false` only meters (settings preview).
    pub fn start(device: Option<&str>, record: bool) -> Result<Self> {
        let shared = Arc::new(Shared {
            samples: Mutex::new(Vec::with_capacity(WHISPER_RATE as usize * 60)),
            level: AtomicU32::new(0),
            recording: AtomicBool::new(record),
            last_voice: Mutex::new(Instant::now()),
        });
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32>>();
        let device = device.map(|s| s.to_string());
        let sh = shared.clone();

        thread::Builder::new().name("vf-capture".into()).spawn(move || {
            let run = || -> Result<(cpal::Stream, u32)> {
                let dev = pick_device(device.as_deref())?;
                let cfg = dev.default_input_config().context("Microphone has no usable format")?;
                let rate = cfg.sample_rate().0;
                let channels = cfg.channels() as usize;
                let sh2 = sh.clone();
                let err = |e| log::error!("audio stream error: {e}");
                let stream = match cfg.sample_format() {
                    cpal::SampleFormat::F32 => dev.build_input_stream(&cfg.into(), move |d: &[f32], _| push(&sh2, d, channels, |s| s), err, None)?,
                    cpal::SampleFormat::I16 => dev.build_input_stream(&cfg.into(), move |d: &[i16], _| push(&sh2, d, channels, |s| s as f32 / 32768.0), err, None)?,
                    cpal::SampleFormat::U16 => dev.build_input_stream(&cfg.into(), move |d: &[u16], _| push(&sh2, d, channels, |s| (s as f32 - 32768.0) / 32768.0), err, None)?,
                    cpal::SampleFormat::I32 => dev.build_input_stream(&cfg.into(), move |d: &[i32], _| push(&sh2, d, channels, |s| s as f32 / 2147483648.0), err, None)?,
                    f => return Err(anyhow!("Unsupported sample format {f:?}")),
                };
                stream.play()?;
                Ok((stream, rate))
            };
            match run() {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        })?;

        let rate = ready_rx.recv_timeout(Duration::from_secs(4)).map_err(|_| anyhow!("Microphone did not respond"))??;
        Ok(Self { shared, stop: stop_tx, rate, started: Instant::now() })
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }

    pub fn since_voice(&self) -> Duration {
        self.shared.last_voice.lock().elapsed()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Stop and return 16 kHz mono samples.
    pub fn finish(self) -> Vec<f32> {
        let _ = self.stop.send(());
        let raw = std::mem::take(&mut *self.shared.samples.lock());
        resample(&raw, self.rate, WHISPER_RATE)
    }

    pub fn cancel(self) {
        let _ = self.stop.send(());
    }
}

fn push<T: Copy>(sh: &Shared, data: &[T], channels: usize, conv: impl Fn(T) -> f32) {
    let mut sum = 0.0f32;
    let frames = data.len() / channels.max(1);
    let rec = sh.recording.load(Ordering::Relaxed);
    let mut buf = if rec { Some(sh.samples.lock()) } else { None };
    for f in 0..frames {
        let mut acc = 0.0;
        for c in 0..channels {
            acc += conv(data[f * channels + c]);
        }
        let s = acc / channels as f32;
        sum += s * s;
        if let Some(b) = buf.as_mut() {
            b.push(s);
        }
    }
    drop(buf);
    if frames > 0 {
        let rms = (sum / frames as f32).sqrt();
        let db = 20.0 * rms.max(1e-7).log10();
        // -55 dBFS is the room, -10 dBFS is close speech.
        let level = ((db + 55.0) / 45.0).clamp(0.0, 1.0);
        sh.level.store(level.to_bits(), Ordering::Relaxed);
        if level > 0.28 {
            *sh.last_voice.lock() = Instant::now();
        }
    }
}

pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    use rubato::{FftFixedIn, Resampler};
    let chunk = 1024;
    let Ok(mut rs) = FftFixedIn::<f32>::new(from as usize, to as usize, chunk, 2, 1) else {
        return linear(input, from, to);
    };
    let mut out = Vec::with_capacity(input.len() * to as usize / from as usize + chunk);
    let mut pos = 0;
    while pos + chunk <= input.len() {
        if let Ok(o) = rs.process(&[&input[pos..pos + chunk]], None) {
            out.extend_from_slice(&o[0]);
        }
        pos += chunk;
    }
    if pos < input.len() {
        if let Ok(o) = rs.process_partial(Some(&[&input[pos..]]), None) {
            out.extend_from_slice(&o[0]);
        }
    }
    if let Ok(o) = rs.process_partial::<&[f32]>(None, None) {
        out.extend_from_slice(&o[0]);
    }
    out
}

fn linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let ratio = from as f64 / to as f64;
    let n = (input.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let j = x as usize;
            let t = (x - j as f64) as f32;
            let a = input[j];
            let b = *input.get(j + 1).unwrap_or(&a);
            a + (b - a) * t
        })
        .collect()
}

/// Trim leading/trailing silence and normalise quiet recordings so short
/// utterances don't get hallucinated into "Thank you." by Whisper.
pub fn prepare(mut s: Vec<f32>) -> Option<Vec<f32>> {
    let win = (WHISPER_RATE / 50) as usize; // 20 ms
    let rms = |w: &[f32]| (w.iter().map(|x| x * x).sum::<f32>() / w.len().max(1) as f32).sqrt();
    let thr = 0.006;
    let voiced: Vec<bool> = s.chunks(win).map(|w| rms(w) > thr).collect();
    let first = voiced.iter().position(|v| *v)?;
    let last = voiced.iter().rposition(|v| *v)?;
    // Need at least ~150 ms of something that looks like speech.
    if voiced.iter().filter(|v| **v).count() < 8 {
        return None;
    }
    let pad = 12; // keep 240 ms either side
    let a = first.saturating_sub(pad) * win;
    let b = ((last + pad + 1) * win).min(s.len());
    s = s[a..b].to_vec();
    let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 0.0 && peak < 0.5 {
        let g = (0.7 / peak).min(8.0);
        s.iter_mut().for_each(|x| *x *= g);
    }
    // Whisper is unhappy below 1 s; pad with silence.
    let min = WHISPER_RATE as usize + WHISPER_RATE as usize / 10;
    if s.len() < min {
        s.resize(min, 0.0);
    }
    Some(s)
}
