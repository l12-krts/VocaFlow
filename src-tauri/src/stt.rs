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
        params.use_gpu(cfg!(target_os = "macos"));
        params.flash_attn(cfg!(target_os = "macos"));
        let ctx = WhisperContext::new_with_params(&path.to_string_lossy(), params)
            .map_err(|e| anyhow!("Couldn't load the speech model ({e})"))?;
        *g = Some((path.to_path_buf(), ctx));
        Ok(())
    }

    pub fn unload(&self) {
        *self.loaded.lock() = None;
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

        let strategy = match req.quality {
            Quality::Low | Quality::Medium => SamplingStrategy::Greedy { best_of: 1 },
            Quality::High => SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 },
            Quality::Ultra => SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 },
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
