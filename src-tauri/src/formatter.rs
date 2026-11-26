//! Context-aware formatting with a small local model (Qwen2.5 1.5B Instruct,
//! Q4_K_M GGUF) on candle. The model only sees what the cleanup rules
//! can't handle: tone, structure, self-corrections. Its output is checked
//! and falls back to the rules-only text if it drifts.

use crate::cleanup;
use crate::context::{Context, Kind};
use anyhow::{anyhow, Result};
use candle_core::{quantized::gguf_file, Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use parking_lot::Mutex;
use std::path::Path;
use std::time::{Duration, Instant};
use tokenizers::Tokenizer;

struct Loaded {
    model: ModelWeights,
    tok: Tokenizer,
    device: Device,
    eos: Vec<u32>,
}

pub struct Formatter {
    loaded: Mutex<Option<Loaded>>,
}

const RULES: &str = "You turn raw speech-to-text into the exact text the speaker meant to type.
Rules:
- Keep the speaker's words, meaning, language and point of view. Never answer, summarize or add information.
- Remove filler words (um, uh, like, you know, I mean, sort of) and false starts.
- Apply self-corrections: if the speaker corrects themselves (\"actually\", \"no wait\", \"I mean\", \"scratch that\"), keep only the corrected version.
- Turn spoken punctuation (\"comma\", \"period\", \"question mark\", \"new line\", \"new paragraph\") into the symbols.
- Fix capitalization, punctuation, numbers and obvious mishearings.
- Never use em dashes or en dashes. Use commas, periods or parentheses.
- Never use emojis.
- Use bullet points only when the speaker lists three or more separate items.
- Output only the final text. No quotes, no explanations.";

fn style(kind: Kind) -> &'static str {
    match kind {
        Kind::Email => "Target: the body of an email. If the speaker said a greeting, put it on its own line followed by a blank line. Use short paragraphs separated by blank lines. If the speaker said a sign-off and name, put them on their own lines at the end. Do not invent a greeting, sign-off or name.",
        Kind::Commit => "Target: a git commit message. First line: imperative mood summary, at most 72 characters, no trailing period. Use a conventional prefix (feat:, fix:, docs:, refactor:, chore:) only if the speaker said one or the surrounding text uses them. If there is more detail, add a blank line and a short body.",
        Kind::Post => "Target: a social media post. Concise and natural, keep the speaker's voice. No hashtags unless spoken. Aim for under 280 characters without dropping meaning.",
        Kind::Chat => "Target: a chat message. Casual and brief. No greeting or sign-off unless spoken. A single paragraph unless the speaker asked for new lines.",
        Kind::Document => "Target: a document. Clean prose in paragraphs.",
        Kind::Code => "Target: a code comment or text in a code editor. Plain, precise wording. Keep identifiers exactly as spoken.",
        Kind::Terminal | Kind::Search => "Target: a short input. Minimal cleanup, no trailing period.",
        Kind::Generic => "Target: a general text field. Light cleanup only, keep it close to what was said.",
    }
}

fn user_message(raw: &str, kind: Kind, ctx: &Context) -> String {
    let mut m = String::new();
    m.push_str(style(kind));
    m.push_str("\n\nWhere: ");
    m.push_str(if ctx.app.is_empty() { "unknown app" } else { &ctx.app });
    if !ctx.title.is_empty() {
        m.push_str(&format!(" (window: \"{}\")", ctx.title.chars().take(120).collect::<String>()));
    }
    if let Some(l) = ctx.label.as_deref().filter(|l| !l.is_empty()) {
        m.push_str(&format!(", field: \"{}\"", l.chars().take(80).collect::<String>()));
    }
    let before = ctx.before.trim();
    if !before.is_empty() {
        let b: String = before.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
        m.push_str(&format!("\nText already in the field before the cursor (do not repeat it):\n\"\"\"\n{b}\n\"\"\""));
    }
    m.push_str(&format!("\n\nSpeech to clean up:\n\"\"\"\n{}\n\"\"\"", raw.trim()));
    m
}

impl Formatter {
    pub fn new() -> Self {
        Self { loaded: Mutex::new(None) }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.lock().is_some()
    }

    pub fn unload(&self) {
        *self.loaded.lock() = None;
    }

    pub fn warm(&self, model: &Path, tokenizer: &Path) -> Result<()> {
        let mut g = self.loaded.lock();
        if g.is_some() {
            return Ok(());
        }
        let device = pick_device();
        let mut file = std::fs::File::open(model)?;
        let content = gguf_file::Content::read(&mut file).map_err(|e| anyhow!("formatter model: {e}"))?;
        let model = ModelWeights::from_gguf(content, &mut file, &device)?;
        let tok = Tokenizer::from_file(tokenizer).map_err(|e| anyhow!("tokenizer: {e}"))?;
        let eos = ["<|im_end|>", "<|endoftext|>"].iter().filter_map(|t| tok.token_to_id(t)).collect();
        *g = Some(Loaded { model, tok, device, eos });
        Ok(())
    }

    /// Format `raw` for the field described by `ctx`. Returns None if the
    /// model output failed the sanity checks (the caller uses rules only).
    pub fn format(&self, raw: &str, kind: Kind, ctx: &Context, budget: Duration) -> Result<Option<String>> {
        let mut g = self.loaded.lock();
        let l = g.as_mut().ok_or_else(|| anyhow!("formatter not loaded"))?;
        let prompt = format!(
            "<|im_start|>system\n{RULES}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            user_message(raw, kind, ctx)
        );
        let enc = l.tok.encode(prompt, false).map_err(|e| anyhow!("{e}"))?;
        let prompt_ids = enc.get_ids().to_vec();
        let raw_tokens = l.tok.encode(raw, false).map(|e| e.len()).unwrap_or(raw.len() / 3);
        let max_new = (raw_tokens * 2 + 48).min(700);

        let started = Instant::now();
        let mut logits_p = LogitsProcessor::from_sampling(0, Sampling::ArgMax);
        let input = Tensor::new(prompt_ids.as_slice(), &l.device)?.unsqueeze(0)?;
        let logits = l.model.forward(&input, 0)?.squeeze(0)?;
        let mut next = logits_p.sample(&logits)?;
        let mut out: Vec<u32> = vec![];
        let mut pos = prompt_ids.len();
        while !l.eos.contains(&next) && out.len() < max_new {
            out.push(next);
            if started.elapsed() > budget {
                log::warn!("formatter over budget, falling back");
                return Ok(None);
            }
            let input = Tensor::new(&[next], &l.device)?.unsqueeze(0)?;
            let logits = l.model.forward(&input, pos)?.squeeze(0)?;
            // Gentle repeat penalty over the recent window: small models
            // occasionally loop on lists.
            let start = out.len().saturating_sub(64);
            let logits = candle_transformers::utils::apply_repeat_penalty(&logits, 1.08, &out[start..])?;
            next = logits_p.sample(&logits)?;
            pos += 1;
        }
        let text = l.tok.decode(&out, true).map_err(|e| anyhow!("{e}"))?;
        log::info!("formatter: {} tokens in {:?}", out.len(), started.elapsed());
        Ok(accept(raw, &text, kind))
    }
}

fn pick_device() -> Device {
    #[cfg(target_os = "macos")]
    if let Ok(d) = Device::new_metal(0) {
        return d;
    }
    Device::Cpu
}

/// Sanity-check model output against the input and enforce house style.
fn accept(raw: &str, out: &str, kind: Kind) -> Option<String> {
    let mut t = out.trim().to_string();
    // Strip wrappers small models like to add.
    for p in ["Here is the cleaned-up text:", "Here's the cleaned up text:", "Cleaned text:", "Output:"] {
        if let Some(rest) = t.strip_prefix(p) {
            t = rest.trim().to_string();
        }
    }
    t = t.trim_matches(|c| c == '"' || c == '`').trim().to_string();
    if t.starts_with("\"\"\"") {
        t = t.trim_start_matches("\"\"\"").trim_end_matches("\"\"\"").trim().to_string();
    }
    let rw = raw.split_whitespace().count().max(1) as f32;
    let ow = t.split_whitespace().count() as f32;
    // Cleanup removes words and emails add line structure, but the model
    // must not summarise or ramble.
    let ratio = ow / rw;
    if t.is_empty() || ratio < 0.4 || ratio > 1.6 + 6.0 / rw {
        log::warn!("formatter output rejected (ratio {ratio:.2})");
        return None;
    }
    let mut t = cleanup::enforce_style(&t);
    if kind == Kind::Commit {
        if let Some(first) = t.lines().next() {
            let trimmed = first.trim_end_matches('.');
            t = std::iter::once(trimmed.to_string()).chain(t.lines().skip(1).map(str::to_string)).collect::<Vec<_>>().join("\n");
        }
    }
    Some(t)
}

/// Whether a dictation is worth a model call.
pub fn worth_formatting(raw: &str, kind: Kind) -> bool {
    let words = raw.split_whitespace().count();
    match kind {
        Kind::Search | Kind::Terminal => false,
        Kind::Email | Kind::Commit => words >= 3,
        _ => words >= 7,
    }
}
