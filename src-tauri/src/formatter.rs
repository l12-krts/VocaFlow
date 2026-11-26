//! Context-aware formatting with a small local model (Qwen2.5 1.5B Instruct,
//! Q4_K_M GGUF) on candle. The model only sees what the cleanup rules
//! can't handle: tone, structure, self-corrections. Its output is checked
//! and falls back to the rules-only text if it drifts.

use crate::cleanup;
use crate::context::{Context, Kind};
use anyhow::{anyhow, Result};
use candle_core::{quantized::gguf_file, Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use crate::qwen2::ModelWeights;
use std::collections::HashMap;
use parking_lot::Mutex;
use std::path::Path;
use std::time::{Duration, Instant};
use tokenizers::Tokenizer;

struct Loaded {
    model: ModelWeights,
    tok: Tokenizer,
    device: Device,
    eos: Vec<u32>,
    /// Model state after the fixed part of the prompt (system + example)
    /// for each target, and its length in tokens. Cloning is cheap: weights
    /// and cache tensors are shared, so each dictation only pays for its
    /// own words.
    prefixes: HashMap<&'static str, (ModelWeights, usize)>,
}

const ALL_KINDS: [Kind; 7] = [Kind::Email, Kind::Chat, Kind::Generic, Kind::Document, Kind::Post, Kind::Commit, Kind::Code];

impl Loaded {
    fn prefix(&mut self, kind: Kind) -> Result<(ModelWeights, usize)> {
        let key = kind.label_key();
        if let Some((m, n)) = self.prefixes.get(key) {
            return Ok((m.clone(), *n));
        }
        let ids = self.tok.encode(prefix_text(kind), false).map_err(|e| anyhow!("{e}"))?.get_ids().to_vec();
        let mut m = self.model.clone();
        let input = Tensor::new(ids.as_slice(), &self.device)?.unsqueeze(0)?;
        m.forward(&input, 0)?;
        self.prefixes.insert(key, (m.clone(), ids.len()));
        Ok((m, ids.len()))
    }
}

pub struct Formatter {
    loaded: Mutex<Option<Loaded>>,
}

const RULES: &str = "You rewrite dictated speech into the exact text the speaker meant to type, formatted for where it is going.
Rules:
- Keep the speaker's words, meaning, language and point of view. Never answer, summarize or add facts.
- Drop filler words and false starts. Apply self-corrections (\"actually\", \"no wait\", \"I mean\") by keeping only the corrected version.
- Fix capitalization, punctuation and obvious mishearings.
- Never use em dashes, en dashes or emojis.
- Use bullet points only for three or more separate items.
- Reply with the final text only.";

fn style(kind: Kind) -> &'static str {
    match kind {
        Kind::Email => "Email body: greeting on its own line, short paragraphs separated by blank lines, sign-off and name on their own lines. Only include a greeting or sign-off if spoken.",
        Kind::Commit => "Git commit message: imperative summary under 72 characters, no trailing period. Optional body after a blank line.",
        Kind::Post => "Social media post: concise, natural, the speaker's voice. No hashtags unless spoken.",
        Kind::Chat => "Chat message: casual and brief, no greeting or sign-off unless spoken.",
        Kind::Document => "Document: clean prose in paragraphs.",
        Kind::Code => "Code comment: plain and precise, identifiers kept exactly.",
        Kind::Terminal | Kind::Search => "Short input: minimal cleanup, no trailing period.",
        Kind::Generic => "General text field: light cleanup, stay close to what was said.",
    }
}

/// One worked example per target. Small models follow a demonstration far
/// more reliably than instructions alone.
fn example(kind: Kind) -> (&'static str, &'static str, &'static str) {
    match kind {
        Kind::Email => (
            "Mail (Re: Budget)",
            "hi sam thanks for the update the numbers look good can we meet tuesday at 3 actually make it 4 best jordan",
            "Hi Sam,\n\nThanks for the update. The numbers look good. Can we meet Tuesday at 4?\n\nBest,\nJordan",
        ),
        Kind::Commit => ("Terminal (git commit)", "fixed the bug where the settings window flashed white when it opened", "Fix settings window flashing white on open"),
        Kind::Post => (
            "X (Home)",
            "just shipped dark mode for the dashboard took way longer than I expected but it looks great",
            "Just shipped dark mode for the dashboard. Took way longer than I expected, but it looks great.",
        ),
        Kind::Chat => ("Slack (#team)", "sounds good I'll push the fix tonight and uh ping you when it's live", "Sounds good, I'll push the fix tonight and ping you when it's live."),
        Kind::Code => ("Code editor", "todo handle the case where the user id is null", "TODO: handle the case where the user ID is null."),
        _ => (
            "Notes",
            "the main risk is timing we need the vendor contract signed by march no wait by february otherwise we slip a quarter",
            "The main risk is timing. We need the vendor contract signed by February, otherwise we slip a quarter.",
        ),
    }
}

fn user_message(where_: &str, before: &str, dictation: &str) -> String {
    let mut m = format!("App: {where_}\n");
    if !before.trim().is_empty() {
        m.push_str(&format!("Already written (context only, do not repeat): {}\n", before.trim()));
    }
    m.push_str(&format!("Dictation: {}", dictation.trim()));
    m
}

fn where_label(ctx: &Context) -> String {
    let app = if ctx.app.is_empty() { "Unknown app" } else { &ctx.app };
    let detail = ctx.label.as_deref().filter(|l| !l.is_empty()).unwrap_or(&ctx.title);
    if detail.is_empty() {
        app.to_string()
    } else {
        format!("{app} ({})", detail.chars().take(80).collect::<String>())
    }
}

/// The fixed part: system prompt, target style and one worked example,
/// ending where the user's turn begins.
fn prefix_text(kind: Kind) -> String {
    let (ex_where, ex_in, ex_out) = example(kind);
    format!(
        "<|im_start|>system\n{RULES}\nTarget: {}<|im_end|>\n\
         <|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n{ex_out}<|im_end|>\n\
         <|im_start|>user\n",
        style(kind),
        user_message(ex_where, "", ex_in),
    )
}

/// The per-dictation part.
fn suffix_text(raw: &str, ctx: &Context) -> String {
    let before: String = {
        let chars: Vec<char> = ctx.before.chars().collect();
        chars[chars.len().saturating_sub(240)..].iter().collect::<String>().replace('\n', " ")
    };
    let _ = before; // the model echoes it; dedupe in `accept` handles overlap
    format!("{}<|im_end|>\n<|im_start|>assistant\n", user_message(&where_label(ctx), "", raw))
}

impl Kind {
    fn label_key(self) -> &'static str {
        match self {
            Kind::Email => "email",
            Kind::Commit => "commit",
            Kind::Post => "post",
            Kind::Chat => "chat",
            Kind::Code => "code",
            Kind::Document => "doc",
            _ => "generic",
        }
    }
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
        {
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
            *g = Some(Loaded { model, tok, device, eos, prefixes: HashMap::new() });
        }
        // Pre-compute each target's prompt prefix so the first dictation
        // into any app is as fast as the hundredth. One at a time, releasing
        // the lock in between, so a dictation never waits for all of them.
        let t = Instant::now();
        for k in ALL_KINDS {
            let mut g = self.loaded.lock();
            if let Some(l) = g.as_mut() {
                if let Err(e) = l.prefix(k) {
                    log::warn!("formatter prefix {k:?}: {e:#}");
                }
            }
        }
        log::info!("formatter prefixes ready in {:?}", t.elapsed());
        Ok(())
    }

    /// Format `raw` for the field described by `ctx`. Returns None if the
    /// model output failed the sanity checks (the caller uses rules only).
    pub fn format(&self, raw: &str, kind: Kind, ctx: &Context, budget: Duration) -> Result<Option<String>> {
        // Busy warming up: don't make the user wait, the rules are fine.
        let Some(mut g) = self.loaded.try_lock_for(Duration::from_secs(3)) else {
            log::warn!("formatter busy, using rules");
            return Ok(None);
        };
        let l = g.as_mut().ok_or_else(|| anyhow!("formatter not loaded"))?;
        let (mut model, plen) = l.prefix(kind)?;
        let enc = l.tok.encode(suffix_text(raw, ctx), false).map_err(|e| anyhow!("{e}"))?;
        let prompt_ids = enc.get_ids().to_vec();
        let raw_tokens = l.tok.encode(raw, false).map(|e| e.len()).unwrap_or(raw.len() / 3);
        let max_new = (raw_tokens * 2 + 48).min(700);

        let started = Instant::now();
        let mut logits_p = LogitsProcessor::from_sampling(0, Sampling::ArgMax);
        let input = Tensor::new(prompt_ids.as_slice(), &l.device)?.unsqueeze(0)?;
        let logits = model.forward(&input, plen)?.squeeze(0)?;
        let mut next = logits_p.sample(&logits)?;
        log::info!("formatter prefill: {} tokens in {:?}", prompt_ids.len(), started.elapsed());
        let mut out: Vec<u32> = vec![];
        let mut pos = plen + prompt_ids.len();
        while !l.eos.contains(&next) && out.len() < max_new {
            out.push(next);
            if started.elapsed() > budget {
                log::warn!("formatter over budget, falling back");
                return Ok(None);
            }
            let input = Tensor::new(&[next], &l.device)?.unsqueeze(0)?;
            let logits = model.forward(&input, pos)?.squeeze(0)?;
            // Gentle repeat penalty over the recent window: small models
            // occasionally loop on lists.
            let start = out.len().saturating_sub(64);
            let logits = candle_transformers::utils::apply_repeat_penalty(&logits, 1.08, &out[start..])?;
            next = logits_p.sample(&logits)?;
            pos += 1;
        }
        let text = l.tok.decode(&out, true).map_err(|e| anyhow!("{e}"))?;
        log::info!("formatter: {} tokens in {:?}", out.len(), started.elapsed());
        Ok(accept(raw, &text, kind, ctx))
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
fn accept(raw: &str, out: &str, kind: Kind, ctx: &Context) -> Option<String> {
    let mut t = out.trim().to_string();
    // Strip wrappers small models like to add: a "Here's the …:" line and
    // "---" fences.
    let lines: Vec<&str> = t.lines().collect();
    if let Some(first) = lines.first() {
        let f = first.trim().to_lowercase();
        if (f.starts_with("here") || f.starts_with("sure") || f.starts_with("cleaned") || f.starts_with("output")) && f.ends_with(':') {
            t = lines[1..].join("\n");
        }
    }
    t = t.lines().filter(|l| l.trim() != "---").collect::<Vec<_>>().join("\n").trim().to_string();
    if let Some(rest) = t.strip_prefix("Dictation:") {
        t = rest.trim().to_string();
    }
    t = t.trim_matches(|c| c == '"' || c == '`').trim().to_string();
    if t.starts_with("\"\"\"") {
        t = t.trim_start_matches("\"\"\"").trim_end_matches("\"\"\"").trim().to_string();
    }
    // Never repeat what's already in the field (a greeting typed before
    // dictating, a subject line): drop leading lines found at its end.
    let norm = |x: &str| x.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let tail = norm(&ctx.before.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect::<String>());
    loop {
        let first = t.lines().next().unwrap_or("").to_string();
        let n = norm(&first);
        if !n.is_empty() && n.len() >= 3 && tail.ends_with(&n) || (!n.is_empty() && tail.contains(&n) && first.len() < 40 && t.lines().count() > 1) {
            t = t.lines().skip(1).collect::<Vec<_>>().join("\n").trim_start().to_string();
        } else {
            break;
        }
    }
    let rw = raw.split_whitespace().count().max(1) as f32;
    let ow = t.split_whitespace().count() as f32;
    // Cleanup removes words and emails add line structure, but the model
    // must not summarise or ramble. Commit summaries are legitimately short.
    let ratio = ow / rw;
    let min = if kind == Kind::Commit { 0.12 } else { 0.4 };
    if t.is_empty() || ratio < min || ratio > 1.6 + 6.0 / rw {
        log::warn!("formatter output rejected (ratio {ratio:.2})");
        return None;
    }
    // Small models sometimes borrow a sign-off or name from the example.
    // Drop trailing lines that share no words with what was said.
    let spoken: std::collections::HashSet<String> =
        raw.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
    loop {
        let lines: Vec<&str> = t.lines().collect();
        let Some(last) = lines.last() else { break };
        let words: Vec<String> = last.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
        if lines.len() > 1 && words.len() <= 4 && !words.is_empty() && !words.iter().any(|w| spoken.contains(w)) {
            t = lines[..lines.len() - 1].join("\n").trim_end().to_string();
        } else {
            break;
        }
    }
    let mut t = cleanup::enforce_style(&t);
    if kind == Kind::Email {
        t = email_spacing(&t);
    }
    if kind == Kind::Commit {
        if let Some(first) = t.lines().next() {
            let trimmed = first.trim_end_matches('.');
            t = std::iter::once(trimmed.to_string()).chain(t.lines().skip(1).map(str::to_string)).collect::<Vec<_>>().join("\n");
        }
    }
    Some(t)
}

/// Blank lines between email paragraphs, but keep a sign-off and the name
/// under it together, and list items together.
fn email_spacing(t: &str) -> String {
    let lines: Vec<&str> = t.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let is_item = |l: &str| l.starts_with("- ") || l.starts_with("• ") || l.split('.').next().is_some_and(|n| !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit()));
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            let prev = lines[i - 1];
            let signoff_name = prev.ends_with(',') && prev.split_whitespace().count() <= 3 && l.split_whitespace().count() <= 3 && i == lines.len() - 1;
            let list = is_item(prev) && is_item(l);
            let into_list = prev.ends_with(':') && is_item(l);
            out.push_str(if signoff_name || list || into_list { "\n" } else { "\n\n" });
        }
        out.push_str(l);
    }
    out
}

/// Whether a dictation is worth a model call.
/// Whether a dictation is worth a model call. The rules already produce
/// clean sentences; the model earns its time where structure matters:
/// emails, commit messages, and long passages that need paragraphs or a list.
pub fn worth_formatting(raw: &str, kind: Kind) -> bool {
    let words = raw.split_whitespace().count();
    match kind {
        Kind::Search | Kind::Terminal => false,
        Kind::Email => words >= 8,
        Kind::Commit => words >= 4,
        Kind::Code => words >= 12,
        _ => words >= 40,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing() {
        assert_eq!(
            email_spacing("Hi Sam,\nThanks for the update.\nWe need:\n1. Copy\n2. Pricing\nBest,\nJordan"),
            "Hi Sam,\n\nThanks for the update.\n\nWe need:\n1. Copy\n2. Pricing\n\nBest,\nJordan"
        );
    }

    #[test]
    fn drops_invented_signoff() {
        let ctx = Context::default();
        let out = accept("we need three things by friday", "We need three things by Friday.\nBest,\nJordan", Kind::Email, &ctx).unwrap();
        assert_eq!(out, "We need three things by Friday.");
    }

    #[test]
    fn drops_repeated_greeting() {
        let ctx = Context { before: "Hi Maria,\n\n".into(), ..Default::default() };
        let out = accept("hi maria thanks for the draft it looks good", "Hi Maria,\n\nThanks for the draft. It looks good.", Kind::Email, &ctx).unwrap();
        assert_eq!(out, "Thanks for the draft. It looks good.");
    }
}
