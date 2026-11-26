//! Model files: one Whisper model per quality tier plus the formatter.
//!
//! Switching tiers downloads the new model in the background while the old
//! one keeps working, then deletes the old file once the new one is
//! verified. Dictation never breaks mid-switch.

use crate::settings::Quality;
use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

const HF: &str = "https://huggingface.co";

pub struct Spec {
    pub file: &'static str,
    pub url: &'static str,
    /// Approximate size, for progress before the server answers.
    pub size: u64,
}

pub fn speech_spec(q: Quality) -> Spec {
    match q {
        Quality::Low => Spec { file: "speech-low.bin", url: "/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin", size: 59_707_625 },
        Quality::Medium => Spec { file: "speech-medium.bin", url: "/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin", size: 190_085_487 },
        Quality::High => Spec { file: "speech-high.bin", url: "/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin", size: 574_041_195 },
        Quality::Ultra => Spec { file: "speech-ultra.bin", url: "/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-q5_0.bin", size: 1_081_140_203 },
    }
}

pub const FORMATTER: [Spec; 1] =
    [Spec { file: "formatter.gguf", url: "/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf", size: 1_117_320_736 }];

pub const ALL_QUALITIES: [Quality; 4] = [Quality::Low, Quality::Medium, Quality::High, Quality::Ultra];

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Status {
    Ready,
    Missing,
    Downloading { received: u64, total: u64 },
    Error { message: String },
}

#[derive(Serialize, Clone, Debug)]
pub struct ModelsState {
    pub speech: Status,
    pub formatter: Status,
}

pub struct Models {
    pub dir: PathBuf,
    pub speech: Mutex<Status>,
    pub formatter: Mutex<Status>,
    /// Bumped whenever the wanted speech tier changes; stale downloads stop.
    speech_gen: AtomicU64,
    formatter_gen: AtomicU64,
}

impl Models {
    pub fn new(dir: PathBuf) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&dir);
        // The GGUF carries its own tokenizer now.
        let _ = std::fs::remove_file(dir.join("formatter-tokenizer.json"));
        // Leftovers from an interrupted download are useless after restart.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "part") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Arc::new(Self {
            dir,
            speech: Mutex::new(Status::Missing),
            formatter: Mutex::new(Status::Missing),
            speech_gen: AtomicU64::new(0),
            formatter_gen: AtomicU64::new(0),
        })
    }

    pub fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    pub fn speech_path(&self, q: Quality) -> PathBuf {
        self.path(speech_spec(q).file)
    }

    pub fn has_speech(&self, q: Quality) -> bool {
        self.speech_path(q).is_file()
    }

    pub fn has_formatter(&self) -> bool {
        FORMATTER.iter().all(|s| self.path(s.file).is_file())
    }

    /// Best model on disk for the wanted tier: the tier itself, otherwise
    /// whatever is still installed from before.
    pub fn usable_speech(&self, wanted: Quality) -> Option<(Quality, PathBuf)> {
        if self.has_speech(wanted) {
            return Some((wanted, self.speech_path(wanted)));
        }
        ALL_QUALITIES.iter().find(|q| self.has_speech(**q)).map(|q| (*q, self.speech_path(*q)))
    }

    pub fn state(&self) -> ModelsState {
        ModelsState { speech: self.speech.lock().clone(), formatter: self.formatter.lock().clone() }
    }

    /// Ensure the speech model for `q` exists; download it if not.
    /// `on_change` is called with throttled progress.
    pub async fn ensure_speech(self: &Arc<Self>, q: Quality, on_change: impl Fn(ModelsState) + Send + Sync + 'static) {
        let gen = self.speech_gen.fetch_add(1, Ordering::SeqCst) + 1;
        if self.has_speech(q) {
            *self.speech.lock() = Status::Ready;
            self.remove_other_speech(q);
            on_change(self.state());
            return;
        }
        let spec = speech_spec(q);
        let me = self.clone();
        let notify = Arc::new(on_change);
        let n2 = notify.clone();
        let result = download(
            &format!("{HF}{}", spec.url),
            &self.path(spec.file),
            spec.size,
            || me.speech_gen.load(Ordering::SeqCst) == gen,
            |received, total| {
                *me.speech.lock() = Status::Downloading { received, total };
                n2(me.state());
            },
        )
        .await;
        if self.speech_gen.load(Ordering::SeqCst) != gen {
            return; // superseded by a newer tier choice
        }
        *self.speech.lock() = match result {
            Ok(()) => {
                self.remove_other_speech(q);
                Status::Ready
            }
            Err(e) => {
                log::error!("speech model download failed: {e:#}");
                Status::Error { message: friendly(&e) }
            }
        };
        notify(self.state());
    }

    pub async fn ensure_formatter(self: &Arc<Self>, on_change: impl Fn(ModelsState) + Send + Sync + 'static) {
        let gen = self.formatter_gen.fetch_add(1, Ordering::SeqCst) + 1;
        if self.has_formatter() {
            *self.formatter.lock() = Status::Ready;
            on_change(self.state());
            return;
        }
        let total: u64 = FORMATTER.iter().map(|s| s.size).sum();
        let done = Arc::new(AtomicU64::new(0));
        let notify = Arc::new(on_change);
        for spec in FORMATTER.iter() {
            if self.path(spec.file).is_file() {
                done.fetch_add(spec.size, Ordering::SeqCst);
                continue;
            }
            let me = self.clone();
            let n2 = notify.clone();
            let base = done.load(Ordering::SeqCst);
            let r = download(
                &format!("{HF}{}", spec.url),
                &self.path(spec.file),
                spec.size,
                || me.formatter_gen.load(Ordering::SeqCst) == gen,
                |received, _| {
                    *me.formatter.lock() = Status::Downloading { received: base + received, total };
                    n2(me.state());
                },
            )
            .await;
            if self.formatter_gen.load(Ordering::SeqCst) != gen {
                return;
            }
            if let Err(e) = r {
                log::error!("formatter download failed: {e:#}");
                *self.formatter.lock() = Status::Error { message: friendly(&e) };
                notify(self.state());
                return;
            }
            done.fetch_add(spec.size, Ordering::SeqCst);
        }
        *self.formatter.lock() = Status::Ready;
        notify(self.state());
    }

    /// Formatting switched off: stop any download in flight. A finished
    /// model stays on disk so switching back on is instant.
    pub fn pause_formatter(&self) {
        self.formatter_gen.fetch_add(1, Ordering::SeqCst);
        for s in FORMATTER.iter() {
            let _ = std::fs::remove_file(self.path(s.file).with_extension("part"));
        }
        let ready = self.has_formatter();
        *self.formatter.lock() = if ready { Status::Ready } else { Status::Missing };
    }

    fn remove_other_speech(&self, keep: Quality) {
        for q in ALL_QUALITIES {
            if q != keep {
                let p = self.speech_path(q);
                if p.exists() {
                    log::info!("removing old speech model {}", p.display());
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }
}

fn friendly(e: &anyhow::Error) -> String {
    let s = format!("{e:#}").to_lowercase();
    if s.contains("space") {
        "Not enough disk space".into()
    } else if s.contains("dns") || s.contains("connect") || s.contains("timed out") || s.contains("network") {
        "Can't reach the download server".into()
    } else {
        "Download failed".into()
    }
}

async fn download(
    url: &str,
    dest: &Path,
    approx: u64,
    alive: impl Fn() -> bool,
    progress: impl Fn(u64, u64),
) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // A stalled stream errors out and the retry resumes with a Range request.
        .read_timeout(Duration::from_secs(30))
        .user_agent(concat!("VocaFlow/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let part = dest.with_extension("part");
    let mut attempt = 0;
    loop {
        attempt += 1;
        match fetch(&client, url, &part, approx, &alive, &progress).await {
            Ok(()) => break,
            Err(e) if attempt < 4 && alive() => {
                log::warn!("download attempt {attempt} failed: {e:#}; retrying");
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(e);
            }
        }
    }
    if !alive() {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(anyhow!("cancelled"));
    }
    tokio::fs::rename(&part, dest).await.context("finalize download")?;
    Ok(())
}

/// Resumable GET into `part`.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    approx: u64,
    alive: &impl Fn() -> bool,
    progress: &impl Fn(u64, u64),
) -> Result<()> {
    let have = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await?.error_for_status()?;
    let resumed = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut received = if resumed { have } else { 0 };
    let total = resp.content_length().map(|l| l + received).unwrap_or(approx);
    let mut file = if resumed {
        tokio::fs::OpenOptions::new().append(true).open(part).await?
    } else {
        tokio::fs::File::create(part).await?
    };
    let mut stream = resp.bytes_stream();
    let mut last = Instant::now() - Duration::from_secs(1);
    progress(received, total);
    while let Some(chunk) = stream.next().await {
        if !alive() {
            return Ok(());
        }
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        if last.elapsed() > Duration::from_millis(120) {
            last = Instant::now();
            progress(received, total);
        }
    }
    file.flush().await?;
    if received < total {
        return Err(anyhow!("connection closed early ({received}/{total})"));
    }
    progress(total, total);
    Ok(())
}
