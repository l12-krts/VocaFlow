//! Whisper (whisper.cpp) transcription. The context for the active tier is
//! loaded once and kept warm; switching tiers swaps it on next use.

use crate::settings::Quality;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Engine {
    loaded: Mutex<Option<(PathBuf, WhisperContext)>>,
    /// Language of the previous dictation: a tie-breaker for the next one.
    last_lang: Mutex<Option<String>>,
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
        Self { loaded: Mutex::new(None), last_lang: Mutex::new(None) }
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

        // Language: an explicit choice, or auto-detect limited to the
        // user's preferred languages. Detection on a few seconds of speech
        // is shaky (short Greek can come out as English, and Whisper then
        // writes English-sounding gibberish), so when it isn't decisive we
        // decode with both top candidates and keep the one Whisper is more
        // confident in. The last language used gets a small head start.
        let candidates: Vec<String> = if req.language != "auto" {
            vec![req.language.to_string()]
        } else {
            let mut c = detect(&mut state, audio, req.preferred, n as usize);
            if let Some(last) = self.last_lang.lock().clone() {
                for (code, p) in c.iter_mut() {
                    if *code == last {
                        *p *= 1.5;
                    }
                }
                c.sort_by(|a, b| b.1.total_cmp(&a.1));
            }
            log::info!("language candidates: {}", c.iter().map(|(l, p)| format!("{l} {p:.2}")).collect::<Vec<_>>().join(", "));
            match c.as_slice() {
                [] => vec!["en".into()],
                [(a, _)] => vec![a.clone()],
                [(a, pa), (b, pb), ..] if *pa < *pb * 4.0 => vec![a.clone(), b.clone()],
                [(a, _), ..] => vec![a.clone()],
            }
        };

        let mut decoded: Vec<(String, f32, String)> = vec![];
        for lang in &candidates {
            let (text, confidence) = self.decode(&mut state, audio, req, lang)?;
            decoded.push((text, confidence, lang.clone()));
        }
        // Wrong-language decodes are either low-confidence gibberish or
        // collapse to a confident "Yeah." covering none of the speech, so
        // confidence counts only as far as the text covers what was said.
        let longest = decoded.iter().map(|(t, _, _)| t.trim().chars().count()).max().unwrap_or(0).max(1) as f32;
        let score = |t: &str, c: f32| c * (t.trim().chars().count() as f32 / (0.6 * longest)).min(1.0);
        if decoded.len() > 1 {
            for (t, c, l) in &decoded {
                log::info!("decoded as {l}: confidence {c:.3}, score {:.3}", score(t, *c));
            }
        }
        let (text, _, lang) = decoded
            .into_iter()
            .max_by(|a, b| score(&a.0, a.1).total_cmp(&score(&b.0, b.1)))
            .ok_or_else(|| anyhow!("no transcript"))?;
        *self.last_lang.lock() = Some(lang.clone());
        Ok(Transcript { text: clean(&text), language: lang })
    }

    /// One decode in `lang`, returning the text and Whisper's mean token
    /// probability (how sure it was).
    fn decode(&self, state: &mut whisper_rs::WhisperState, audio: &[f32], req: &Request, lang: &str) -> Result<(String, f32)> {
        // Beam search buys a little accuracy for a lot of compute: worth it
        // on a GPU, not on a laptop CPU.
        let gpu = gpu_name().is_some();
        let strategy = match (req.quality, gpu) {
            (Quality::Low | Quality::Medium, _) | (_, false) => SamplingStrategy::Greedy { best_of: 1 },
            (Quality::High, true) => SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 },
            (Quality::Ultra, true) => SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 },
        };
        let mut p = FullParams::new(strategy);
        p.set_n_threads(threads());
        p.set_language(Some(lang));
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
        let (mut sum, mut count) = (0.0f32, 0usize);
        for seg in state.as_iter() {
            // Drop segments Whisper itself thinks are silence.
            if seg.no_speech_probability() > 0.75 {
                continue;
            }
            if let Ok(s) = seg.to_str_lossy() {
                text.push_str(&s);
            }
            for i in 0..seg.n_tokens() {
                if let Some(tok) = seg.get_token(i) {
                    let special = tok.to_str_lossy().map(|t| t.starts_with("[_") || t.starts_with("<|")).unwrap_or(true);
                    if !special {
                        sum += tok.token_probability();
                        count += 1;
                    }
                }
            }
        }
        Ok((text, if count == 0 { 0.0 } else { sum / count as f32 }))
    }
}

/// Preferred languages (or, without preferences, the top languages) with
/// their detection probabilities, best first.
fn detect(state: &mut whisper_rs::WhisperState, audio: &[f32], preferred: &[String], threads: usize) -> Vec<(String, f32)> {
    if state.pcm_to_mel(audio, threads).is_err() {
        return vec![];
    }
    let Ok((_, probs)) = state.lang_detect(0, threads) else { return vec![] };
    let mut out: Vec<(String, f32)> = if preferred.is_empty() {
        probs
            .iter()
            .enumerate()
            .filter_map(|(id, p)| whisper_rs::get_lang_str(id as i32).map(|l| (l.to_string(), *p)))
            .collect()
    } else {
        preferred
            .iter()
            .filter_map(|code| whisper_rs::get_lang_id(code).map(|id| (code.clone(), probs.get(id as usize).copied().unwrap_or(0.0))))
            .collect()
    };
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.truncate(3);
    out
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


/// Whisper treats the initial prompt as "previous text": a bare list of
/// the words biases spelling. No English around it, or Whisper leans
/// towards English (and translating) for other languages.
fn prompt_for(words: &[String]) -> String {
    if words.is_empty() {
        return String::new();
    }
    let list: Vec<&str> = words.iter().map(|s| s.as_str()).take(60).collect();
    format!("{}.", list.join(", "))
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
