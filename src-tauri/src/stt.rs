//! Whisper (whisper.cpp) transcription. The context for the active tier is
//! loaded once and kept warm; switching tiers swaps it on next use.

use crate::settings::Quality;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Engine {
    loaded: Mutex<Option<(PathBuf, WhisperContext)>>,
}

pub struct Request<'a> {
    pub language: &'a str,
    pub preferred: &'a [String],
    pub dictionary: &'a [String],
    pub quality: Quality,
}

pub struct Transcript {
    pub text: String,
    pub language: String,
}

/// The GPU whisper.cpp will use, if any.
pub fn gpu_name() -> Option<String> {
    use whisper_rs::whisper_rs_sys as sys;
    static NAME: once_cell::sync::OnceCell<Option<String>> = once_cell::sync::OnceCell::new();
    NAME.get_or_init(|| unsafe {
        for i in 0..sys::ggml_backend_dev_count() {
            let dev = sys::ggml_backend_dev_get(i);
            if sys::ggml_backend_dev_type(dev) == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU {
                let d = sys::ggml_backend_dev_description(dev);
                if !d.is_null() {
                    return Some(std::ffi::CStr::from_ptr(d).to_string_lossy().trim().to_string());
                }
            }
        }
        None
    })
    .clone()
}

fn threads() -> i32 {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8) as i32
}

impl Engine {
    pub fn new() -> Self {
        Self { loaded: Mutex::new(None) }
    }

    /// Load the model ahead of time so the first dictation isn't slow.
    pub fn warm(&self, path: &Path) -> Result<()> {
        let mut g = self.loaded.lock();
        if g.as_ref().is_some_and(|(p, _)| p == path) {
            return Ok(());
        }
        *g = None; // free the old model before loading a large new one
        let mut params = WhisperContextParameters::default();
        // Metal on macOS, Vulkan elsewhere. whisper.cpp falls back to the
        // CPU by itself when there's no usable GPU.
        params.use_gpu(true);
        params.flash_attn(cfg!(target_os = "macos"));
        let ctx = WhisperContext::new_with_params(&path.to_string_lossy(), params)
            .map_err(|e| anyhow!("Couldn't load the speech model ({e})"))?;
        // One silent pass compiles the GPU pipelines and touches every
        // weight, so the user's first dictation isn't the slow one.
        let t = std::time::Instant::now();
        if let Ok(mut state) = ctx.create_state() {
            let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            p.set_n_threads(threads());
            p.set_language(Some("en"));
            p.set_no_context(true);
            p.set_single_segment(true);
            p.set_print_progress(false);
            p.set_print_realtime(false);
            p.set_print_special(false);
            p.set_print_timestamps(false);
            p.set_audio_ctx(audio_ctx(16_000));
            let _ = state.full(p, &vec![0.0f32; 16_000]);
        }
        log::info!("speech model warm in {:?} ({})", t.elapsed(), gpu_name().unwrap_or_else(|| "CPU".into()));
        *g = Some((path.to_path_buf(), ctx));
        Ok(())
    }

    pub fn transcribe(&self, path: &Path, audio: &[f32], req: &Request) -> Result<Transcript> {
        self.warm(path)?;
        let g = self.loaded.lock();
        let (_, ctx) = g.as_ref().ok_or_else(|| anyhow!("model not loaded"))?;
        let mut state = ctx.create_state().map_err(|e| anyhow!("{e}"))?;
        let n = threads();

        // Language: explicit choice, or auto-detect limited to the user's
        // preferred languages (a big accuracy win for bilingual speakers,
        // where free auto-detect flips on short phrases).
        let lang: String = if req.language != "auto" {
            req.language.to_string()
        } else {
            detect(&mut state, audio, req.preferred, n as usize).unwrap_or_else(|| "en".into())
        };

        // Beam search buys a little accuracy for a lot of compute: worth it
        // on a GPU, not on a laptop CPU.
        let gpu = gpu_name().is_some();
        let strategy = match (req.quality, gpu) {
            (Quality::Low | Quality::Medium, _) | (_, false) => SamplingStrategy::Greedy { best_of: 1 },
            (Quality::High, true) => SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 },
            (Quality::Ultra, true) => SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 },
        };
        let mut p = FullParams::new(strategy);
        p.set_n_threads(n);
        p.set_language(Some(&lang));
        p.set_translate(false);
        p.set_no_context(true);
        p.set_no_timestamps(true);
        p.set_single_segment(audio.len() < 16_000 * 25);
        p.set_suppress_blank(true);
        p.set_suppress_nst(true);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_special(false);
        p.set_print_timestamps(false);
        p.set_temperature(0.0);
        p.set_audio_ctx(audio_ctx(audio.len()));
        let prompt = prompt_for(req.dictionary);
        if !prompt.is_empty() {
            p.set_initial_prompt(&prompt);
        }

        state.full(p, audio).map_err(|e| anyhow!("Transcription failed ({e})"))?;
        let mut text = String::new();
        for seg in state.as_iter() {
            // Drop segments Whisper itself thinks are silence.
            if seg.no_speech_probability() > 0.75 {
                continue;
            }
            if let Ok(s) = seg.to_str_lossy() {
                text.push_str(&s);
            }
        }
        Ok(Transcript { text: clean(&text), language: lang })
    }
}

/// Whisper always encodes a 30 s window (1500 frames), so a two-second
/// phrase costs as much as a long paragraph. Encoding only what we have,
/// plus generous headroom, makes short dictations several times faster
/// without hurting accuracy. Long recordings use the full window.
fn audio_ctx(samples: usize) -> i32 {
    let secs = samples as f32 / 16_000.0;
    if secs >= 24.0 {
        return 0; // 0 = model default (1500)
    }
    let frames = ((secs + 1.5) * 50.0 * 1.15) as i32;
    // Round up to a multiple of 64; never go below ~8 s of context.
    (((frames + 63) / 64) * 64).clamp(448, 1500)
}

fn detect(state: &mut whisper_rs::WhisperState, audio: &[f32], preferred: &[String], threads: usize) -> Option<String> {
    state.pcm_to_mel(audio, threads).ok()?;
    let (best, probs) = state.lang_detect(0, threads).ok()?;
    if preferred.is_empty() {
        return whisper_rs::get_lang_str(best).map(str::to_string);
    }
    preferred
        .iter()
        .filter_map(|code| whisper_rs::get_lang_id(code).map(|id| (code, probs.get(id as usize).copied().unwrap_or(0.0))))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c.clone())
}

/// Whisper treats the initial prompt as "previous text", so a natural
/// sentence listing the words biases spelling without leaking into output.
fn prompt_for(words: &[String]) -> String {
    if words.is_empty() {
        return String::new();
    }
    let list: Vec<&str> = words.iter().map(|s| s.as_str()).take(60).collect();
    format!("Glossary: {}.", list.join(", "))
}

/// Known Whisper hallucinations on silence or noise.
const HALLUCINATIONS: &[&str] = &[
    "thank you.", "thanks for watching!", "thank you for watching.", "thanks for watching.", "you", "bye.",
    "[blank_audio]", "(silence)", "[silence]", "[music]", "(music)", "subtitles by the amara.org community",
];

fn clean(s: &str) -> String {
    let t = s.trim();
    if HALLUCINATIONS.iter().any(|h| t.eq_ignore_ascii_case(h)) {
        return String::new();
    }
    // Strip bracketed non-speech tags like [Music] or (laughs).
    let re = regex::Regex::new(r"\s*[\[(][^\])]{1,30}[\])]\s*").unwrap();
    let t = re.replace_all(t, " ");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}
