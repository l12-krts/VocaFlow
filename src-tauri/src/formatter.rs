//! Context-aware formatting with a small local model (Qwen2.5 1.5B
//! Instruct, Q4_K_M) served by a bundled `llama-server` on the GPU: Vulkan
//! on Windows and Linux (Intel, AMD, NVIDIA), Metal on macOS.
//!
//! It runs as a child process rather than in-process because whisper.cpp
//! and llama.cpp each embed their own ggml, which can't be linked twice. A
//! separate process also keeps the 1 GB model's memory out of the app and
//! is cleaned up by the OS if VocaFlow ever dies.
//!
//! The model only does what rules can't (structure and tone); its output
//! is checked and falls back to the rules-only text if it drifts.

use crate::cleanup;
use crate::context::{Context, Kind};
use anyhow::{anyhow, Context as _, Result};
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    port: u16,
    key: String,
}

pub struct Formatter {
    server: Mutex<Option<Server>>,
    /// Folder holding llama-server and its libraries.
    dir: Mutex<Option<PathBuf>>,
    log: Mutex<Option<PathBuf>>,
    /// GPU the server reported, e.g. "Intel(R) Iris(R) Xe Graphics".
    pub gpu: Arc<Mutex<Option<String>>>,
    /// Measured generation speed (tokens/s), to skip the model when a long
    /// dictation could never finish in time on this machine.
    speed: Mutex<Option<f64>>,
}

/// Context window. Dictations are processed in chunks well under this, so
/// prompt + chunk + rewrite always fit with room to spare.
const N_CTX: usize = 4096;
/// Largest piece of dictation sent in one request, in tokens: about 250
/// English or 120 Greek words (non-Latin scripts take 2-4 tokens a word).
/// A small model rewrites short passages faithfully; given a whole long
/// email at once it starts quietly dropping sentences.
const CHUNK_TOKENS: usize = 320;
/// Safety margin kept free in the context.
const CTX_MARGIN: usize = 96;

const RULES: &str = "You clean up dictated speech into the exact text the speaker meant to type, formatted for where it is going.
Rules:
- Never translate. Write in the language the speaker used and keep every word in its original language. If the dictation mixes languages, keep each part exactly as spoken.
- Keep the speaker's words, meaning and point of view. Never answer, summarize, shorten or add facts.
- Drop filler words and false starts. When the speaker corrects themselves, keep only the corrected version.
- Fix capitalization, punctuation and obvious mishearings.
- Never use em dashes, en dashes or emojis.
- Use bullet points only for three or more separate items.
- Reply with the final text only.";

fn style(kind: Kind) -> &'static str {
    match kind {
        Kind::Email => "Email body: short paragraphs separated by blank lines. The greeting and sign-off are added separately, so never add either.",
        Kind::Commit => "Git commit message: imperative summary under 72 characters, no trailing period. Optional body after a blank line.",
        Kind::Post => "Social media post: concise, natural, the speaker's voice. No hashtags unless spoken.",
        Kind::Chat => "Chat message: casual and brief, no greeting or sign-off unless spoken.",
        Kind::Document => "Document: clean prose in paragraphs.",
        Kind::Code => "Code comment: plain and precise, identifiers kept exactly.",
        Kind::Terminal | Kind::Search => "Short input: minimal cleanup, no trailing period.",
        Kind::Generic => "General text field: light cleanup, stay close to what was said.",
    }
}

/// English name of a Whisper language code, for the prompt.
pub fn language_name(code: &str) -> &'static str {
    match code {
        "en" => "English", "el" => "Greek", "es" => "Spanish", "fr" => "French", "de" => "German", "it" => "Italian",
        "pt" => "Portuguese", "nl" => "Dutch", "pl" => "Polish", "ru" => "Russian", "uk" => "Ukrainian", "cs" => "Czech",
        "sv" => "Swedish", "da" => "Danish", "no" => "Norwegian", "fi" => "Finnish", "ro" => "Romanian", "hu" => "Hungarian",
        "bg" => "Bulgarian", "hr" => "Croatian", "sr" => "Serbian", "sk" => "Slovak", "sl" => "Slovenian", "tr" => "Turkish",
        "ar" => "Arabic", "he" => "Hebrew", "fa" => "Persian", "hi" => "Hindi", "ur" => "Urdu", "bn" => "Bengali",
        "ta" => "Tamil", "zh" => "Chinese", "ja" => "Japanese", "ko" => "Korean", "vi" => "Vietnamese", "th" => "Thai",
        "id" => "Indonesian", "ms" => "Malay", "tl" => "Tagalog", "ca" => "Catalan", "lt" => "Lithuanian", "lv" => "Latvian",
        "et" => "Estonian", "sw" => "Swahili", "af" => "Afrikaans",
        _ => "",
    }
}

type Example = (&'static str, &'static str, &'static str);

/// One worked example per target, in the dictation's language. Small
/// models follow a demonstration far more reliably than instructions, and
/// an English example on Greek speech is an invitation to translate, so
/// other languages get no example rather than an English one.
fn example(kind: Kind, lang: &str) -> Option<Example> {
    match lang {
        "en" => Some(match kind {
            Kind::Email => (
                "Mail (Re: Budget)",
                "thanks for the update the numbers look good. i went through the forecast too and the q3 dip makes sense. can we meet tuesday at 4 to go over the hiring plan",
                "Thanks for the update, the numbers look good.\n\nI went through the forecast too and the Q3 dip makes sense. Can we meet Tuesday at 4 to go over the hiring plan?",
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
        }),
        "el" => match kind {
            Kind::Email => Some((
                "Mail (Re: Προσχέδιο)",
                "ευχαριστώ για το προσχέδιο φαίνεται πολύ καλό. το διάβασα και το πρωί και η δομή στέκει. μπορούμε να τα πούμε την τρίτη στις τέσσερις",
                "Ευχαριστώ για το προσχέδιο, φαίνεται πολύ καλό.\n\nΤο διάβασα και το πρωί και η δομή στέκει. Μπορούμε να τα πούμε την Τρίτη στις τέσσερις;",
            )),
            Kind::Chat => Some((
                "Slack (#ομάδα)",
                "εντάξει θα ανεβάσω τη διόρθωση απόψε και θα σου πω όταν είναι έτοιμη",
                "Εντάξει, θα ανεβάσω τη διόρθωση απόψε και θα σου πω όταν είναι έτοιμη.",
            )),
            Kind::Post => Some((
                "X (Home)",
                "μόλις κυκλοφόρησε το σκοτεινό θέμα στην εφαρμογή πήρε περισσότερο χρόνο απ' ό,τι περίμενα αλλά αξίζει",
                "Μόλις κυκλοφόρησε το σκοτεινό θέμα στην εφαρμογή. Πήρε περισσότερο χρόνο απ' ό,τι περίμενα, αλλά αξίζει.",
            )),
            Kind::Commit | Kind::Code => None,
            _ => Some((
                "Notes",
                "ο βασικός κίνδυνος είναι ο χρόνος πρέπει να υπογραφεί η σύμβαση μέχρι τον μάρτιο αλλιώς καθυστερούμε ένα τρίμηνο",
                "Ο βασικός κίνδυνος είναι ο χρόνος. Πρέπει να υπογραφεί η σύμβαση μέχρι τον Μάρτιο, αλλιώς καθυστερούμε ένα τρίμηνο.",
            )),
        },
        _ => None,
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

fn language_rule(lang: &str) -> String {
    match language_name(lang) {
        "" => "Language: reply in the exact language of the dictation. Do not translate.".to_string(),
        name => format!("Language: the dictation is in {name}. Reply in {name} only. Do not translate any part of it."),
    }
}

/// The fixed part: system prompt, target, language and (maybe) a worked
/// example, ending where the user's turn begins. llama-server caches it
/// between requests.
fn prefix_text(kind: Kind, lang: &str) -> String {
    let mut p = format!("<|im_start|>system\n{RULES}\nTarget: {}\n{}<|im_end|>\n", style(kind), language_rule(lang));
    if let Some((ex_where, ex_in, ex_out)) = example(kind, lang) {
        p.push_str(&format!("<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n{ex_out}<|im_end|>\n", user_message(ex_where, "", ex_in)));
    }
    p.push_str("<|im_start|>user\n");
    p
}

/// The per-dictation part. The language reminder sits right before the
/// answer, where a small model pays the most attention.
fn suffix_text(text: &str, ctx: &Context, lang: &str, part: Option<(usize, usize)>) -> String {
    let mut m = format!("App: {}\n", where_label(ctx));
    if let Some((i, n)) = part {
        let pos = if i == 1 { "the beginning" } else if i == n { "the end" } else { "a middle part" };
        m.push_str(&format!("This is part {i} of {n} of one long dictation ({pos}). Format only this part and continue naturally; no greeting or sign-off unless spoken in it.\n"));
    }
    m.push_str(&format!("Dictation: {}\n", text.trim()));
    m.push_str(match language_name(lang) {
        "" => "(Keep the original language. Do not translate.)",
        _ => "",
    });
    if !language_name(lang).is_empty() {
        m.push_str(&format!("(Reply in {}. Do not translate.)", language_name(lang)));
    }
    format!("{m}<|im_end|>\n<|im_start|>assistant\n")
}

#[allow(dead_code)]
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
        Self {
            server: Mutex::new(None),
            dir: Mutex::new(None),
            log: Mutex::new(None),
            gpu: Arc::new(Mutex::new(None)),
            speed: Mutex::new(None),
        }
    }

    /// Where the bundled server lives, and where its log goes.
    pub fn configure(&self, dir: PathBuf, log: PathBuf) {
        *self.dir.lock() = Some(dir);
        *self.log.lock() = Some(log);
    }

    pub fn is_loaded(&self) -> bool {
        let mut g = self.server.lock();
        match g.as_mut() {
            Some(s) => matches!(s.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn unload(&self) {
        if let Some(mut s) = self.server.lock().take() {
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
    }

    fn binary(&self) -> Option<PathBuf> {
        let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
        if let Some(p) = std::env::var_os("VOCAFLOW_LLAMA_SERVER").map(PathBuf::from).filter(|p| p.is_file()) {
            return Some(p);
        }
        if let Some(p) = self.dir.lock().as_ref().map(|d| d.join(exe)).filter(|p| p.is_file()) {
            return Some(p);
        }
        std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(exe)).find(|p| p.is_file()))
    }

    /// Start the server with the model and wait until it answers.
    pub fn warm(&self, model: &Path) -> Result<()> {
        if self.is_loaded() {
            return Ok(());
        }
        self.unload();
        let bin = self.binary().ok_or_else(|| anyhow!("llama-server not found"))?;
        let dir = bin.parent().map(Path::to_path_buf).unwrap_or_default();
        let port = free_port()?;
        let key = format!("{:032x}", rand::random::<u128>());
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).div_ceil(2).max(2);

        let mut cmd = Command::new(&bin);
        cmd.arg("-m").arg(model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--api-key", &key])
            // Every layer on the GPU; ignored when there's no GPU backend.
            .args(["-ngl", "999"])
            // Dictations are short: a 2k context keeps the KV cache small.
            .args(["-c", &N_CTX.to_string(), "-np", "1", "-t", &threads.to_string()])
            .args(["--no-webui", "--cache-reuse", "256"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // The server's own libraries sit next to it.
        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", join_env("LD_LIBRARY_PATH", &dir));
        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", join_env("DYLD_LIBRARY_PATH", &dir));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.env("PATH", join_env("PATH", &dir));
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            // Die with VocaFlow, even if it crashes.
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let mut child = spawn(cmd).with_context(|| format!("start {}", bin.display()))?;
        #[cfg(windows)]
        crate::platform::windows::kill_with_parent(&child);

        // Keep a log for support, and pick up which GPU it chose.
        if let Some(stderr) = child.stderr.take() {
            let log = self.log.lock().clone();
            let gpu = self.gpu.clone();
            std::thread::spawn(move || {
                let mut file = log.and_then(|p| std::fs::File::create(p).ok());
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Some(f) = file.as_mut() {
                        let _ = writeln!(f, "{line}");
                    }
                    if let Some(name) = gpu_from_log(&line) {
                        gpu.lock().get_or_insert(name);
                    }
                }
            });
        }

        let client = reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build()?;
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(anyhow!("llama-server exited ({status})"));
            }
            let ok = client
                .get(format!("http://127.0.0.1:{port}/health"))
                .bearer_auth(&key)
                .send()
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if ok {
                break;
            }
            if started.elapsed() > Duration::from_secs(180) {
                let _ = child.kill();
                return Err(anyhow!("llama-server did not become ready"));
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        log::info!("formatter ready in {:?} on {}", started.elapsed(), self.gpu.lock().clone().unwrap_or_else(|| "CPU".into()));
        *self.server.lock() = Some(Server { child, port, key });

        // Prime the prompt cache with the most common target so the first
        // email is as fast as the tenth. This
        // also measures this machine's generation speed for the time limits.
        let probe = format!("{}{}", prefix_text(Kind::Email, "en"), suffix_text("thanks for the update the numbers look good see you tomorrow", &Context::default(), "en", None));
        let _ = self.complete(&probe, 24, Duration::from_secs(30));
        Ok(())
    }

    fn endpoint(&self) -> Result<(u16, String)> {
        let g = self.server.lock();
        let s = g.as_ref().ok_or_else(|| anyhow!("formatter not running"))?;
        Ok((s.port, s.key.clone()))
    }

    /// Exact token count from the model's own tokenizer.
    fn count_tokens(&self, text: &str) -> Result<usize> {
        let (port, key) = self.endpoint()?;
        let client = reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build()?;
        let v: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/tokenize"))
            .bearer_auth(&key)
            .json(&serde_json::json!({ "content": text, "add_special": false }))
            .send()?
            .error_for_status()?
            .json()?;
        Ok(v.get("tokens").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0))
    }

    /// One completion. None = timed out or cut off (never paste half a
    /// sentence).
    fn complete(&self, prompt: &str, n_predict: usize, budget: Duration) -> Result<Option<String>> {
        let (port, key) = self.endpoint()?;
        let client = reqwest::blocking::Client::builder().no_proxy().timeout(budget + Duration::from_secs(3)).build()?;
        let body = serde_json::json!({
            "prompt": prompt,
            "n_predict": n_predict,
            "temperature": 0.0,
            "top_k": 1,
            "repeat_penalty": 1.08,
            "repeat_last_n": 64,
            "cache_prompt": true,
            "stop": ["<|im_end|>", "<|endoftext|>"],
            "t_max_predict_ms": budget.as_millis() as u64,
            "stream": false,
        });
        let resp = match client.post(format!("http://127.0.0.1:{port}/completion")).bearer_auth(&key).json(&body).send() {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let v: serde_json::Value = resp.error_for_status()?.json()?;
        if let Some(tps) = v.get("timings").and_then(|t| t.get("predicted_per_second")).and_then(|x| x.as_f64()) {
            if tps.is_finite() && tps > 0.0 && n_predict > 8 {
                *self.speed.lock() = Some(tps);
            }
        }
        // Stopped by a limit (time or length) rather than finishing: the
        // text is incomplete.
        let limited = v.get("stop_type").and_then(|s| s.as_str()) == Some("limit")
            || v.get("stopped_limit").and_then(|b| b.as_bool()) == Some(true);
        if limited && n_predict > 1 {
            return Ok(None);
        }
        Ok(v.get("content").and_then(|c| c.as_str()).map(str::to_string))
    }

    /// Format `text` (already rule-cleaned) for the field in `ctx`, keeping
    /// it in `lang`. Long dictations are formatted in sentence-aligned
    /// chunks that always fit the context; any chunk the model can't do in
    /// time or gets wrong keeps its rule-cleaned text. Returns None when
    /// nothing came back from the model.
    pub fn format(&self, text: &str, kind: Kind, ctx: &Context, lang: &str) -> Result<Option<String>> {
        // Emails: the model only sees the body. Greeting and sign-off are
        // put back exactly as spoken, so they can't be lost or invented.
        if kind == Kind::Email {
            let (greeting, body, signoff) = cleanup::split_email(text);
            if body.trim().is_empty() {
                return Ok(None);
            }
            let formatted = self.format_body(&body, kind, ctx, lang)?;
            return Ok(formatted.map(|b| email_spacing(&cleanup::join_email(greeting.as_deref(), &b, signoff.as_deref()))));
        }
        self.format_body(text, kind, ctx, lang)
    }

    fn format_body(&self, text: &str, kind: Kind, ctx: &Context, lang: &str) -> Result<Option<String>> {
        let started = Instant::now();
        let prefix = prefix_text(kind, lang);
        let prefix_tokens = self.count_tokens(&prefix).unwrap_or(prefix.chars().count() / 2);
        let total = self.count_tokens(text).unwrap_or(text.chars().count() / 2).max(1);
        let chunks = split_chunks(text, total, CHUNK_TOKENS);
        let n = chunks.len();

        let mut out: Vec<String> = Vec::with_capacity(n);
        let mut any = false;
        let mut before = ctx.before.clone();
        // Nobody should wait minutes for "Writing email…": past the
        // deadline the remaining parts keep their rule-cleaned text.
        let deadline = Duration::from_secs(if std::env::var_os("VOCAFLOW_FORMAT_NO_TIMEOUT").is_some() { 3600 } else { 40 });
        // Known up front that it can't finish? Don't make the user wait for
        // a partial attempt: the rules are instant.
        if let Some(tps) = *self.speed.lock() {
            let expected = Duration::from_secs_f64(total as f64 * 1.1 / tps);
            if expected > deadline {
                log::info!("formatter: {total} tokens would take ~{expected:?} at {tps:.0} tok/s, using rules");
                return Ok(None);
            }
        }
        for (i, chunk) in chunks.iter().enumerate() {
            let left = deadline.saturating_sub(started.elapsed());
            if let Some(tps) = *self.speed.lock() {
                let expected = Duration::from_secs_f64(chunk.chars().count() as f64 / text.chars().count().max(1) as f64 * total as f64 / tps);
                if expected > left {
                    log::warn!("formatter: out of time at part {}/{n}, rules for the rest", i + 1);
                    out.extend(chunks[i..].iter().cloned());
                    break;
                }
            }
            if left < Duration::from_secs(2) {
                log::warn!("formatter: out of time at part {}/{n}, rules for the rest", i + 1);
                out.extend(chunks[i..].iter().cloned());
                break;
            }
            let tokens = ((chunk.chars().count() as f64 / text.chars().count().max(1) as f64) * total as f64).ceil() as usize + 8;
            // Room for the rewrite: a bit longer than the input (structure,
            // punctuation), never past the context.
            let room = N_CTX.saturating_sub(prefix_tokens + tokens + 120 + CTX_MARGIN);
            let max_new = (tokens * 3 / 2 + 64).min(room);
            // Time: what this part should need at the measured speed, never
            // more than what's left of the overall deadline.
            let Some(budget) = chunk_budget(max_new.min(tokens + 64), *self.speed.lock()) else {
                log::warn!("formatter: part {}/{n} would take too long on this machine, rules for the rest", i + 1);
                out.extend(chunks[i..].iter().cloned());
                break;
            };
            let budget = budget.min(left);
            let part = (n > 1).then_some((i + 1, n));
            let prompt = format!("{prefix}{}", suffix_text(chunk, ctx, lang, part));
            let part_ctx = Context { before: before.clone(), ..ctx.clone() };
            let result = match self.complete(&prompt, max_new, budget) {
                Ok(Some(t)) => accept(chunk, &t, kind, &part_ctx).map(|t| if lang == "en" { t } else { restore_words(chunk, &t) }),
                Ok(None) => {
                    // Too slow here: don't make the user wait through every
                    // remaining part timing out too.
                    log::warn!("formatter: part {}/{n} over budget, rules for the rest", i + 1);
                    out.extend(chunks[i..].iter().cloned());
                    break;
                }
                Err(e) => {
                    log::error!("formatter: part {}/{n}: {e:#}", i + 1);
                    None
                }
            };
            let piece = match result {
                Some(t) => {
                    any = true;
                    t
                }
                None => chunk.clone(),
            };
            before = piece.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
            out.push(piece);
        }
        log::info!("formatter: {n} part(s), {total} tokens, {:?}", started.elapsed());
        if !any {
            return Ok(None);
        }
        let sep = match kind {
            Kind::Email | Kind::Document | Kind::Generic | Kind::Code => "\n\n",
            _ => " ",
        };
        Ok(Some(out.join(sep)))
    }
}

/// Time allowed for one request generating up to `tokens` tokens. None
/// when, at the measured speed, it can't possibly finish in 45 s.
fn chunk_budget(tokens: usize, speed: Option<f64>) -> Option<Duration> {
    // Test harness only: measure quality on slow machines without timeouts.
    if std::env::var_os("VOCAFLOW_FORMAT_NO_TIMEOUT").is_some() {
        return Some(Duration::from_secs(1800));
    }
    let secs = match speed {
        Some(tps) => {
            let expected = tokens as f64 / tps;
            if expected > 45.0 {
                return None;
            }
            (expected * 1.5 + 3.0).clamp(6.0, 60.0)
        }
        // First run: generous, the measurement comes from this request.
        None => (3.0 + tokens as f64 * 0.05).clamp(8.0, 60.0),
    };
    Some(Duration::from_secs_f64(secs))
}

/// Split at sentence ends into pieces of at most ~`max_tokens` tokens,
/// using the text's own chars-per-token ratio. Run-on dictation without
/// punctuation is split between words.
pub fn split_chunks(text: &str, total_tokens: usize, max_tokens: usize) -> Vec<String> {
    let chars = text.chars().count().max(1);
    if total_tokens <= max_tokens {
        return vec![text.trim().to_string()];
    }
    let max_chars = ((max_tokens as f64 / total_tokens as f64) * chars as f64).max(40.0) as usize;

    // Sentences, keeping their punctuation and paragraph breaks.
    let mut sentences: Vec<String> = vec![];
    let mut cur = String::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        cur.push(c);
        let end = matches!(c, '.' | '!' | '?' | ';' | '\n' | '\u{37e}');
        if end && it.peek().map_or(true, |n| n.is_whitespace()) {
            sentences.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        sentences.push(cur);
    }
    // Over-long sentences: split between words.
    let mut pieces: Vec<String> = vec![];
    for snt in sentences {
        if snt.chars().count() <= max_chars {
            pieces.push(snt);
            continue;
        }
        let mut part = String::new();
        for w in snt.split_inclusive(char::is_whitespace) {
            if part.chars().count() + w.chars().count() > max_chars && !part.trim().is_empty() {
                pieces.push(std::mem::take(&mut part));
            }
            part.push_str(w);
        }
        if !part.trim().is_empty() {
            pieces.push(part);
        }
    }
    // Pack pieces into chunks.
    let mut chunks: Vec<String> = vec![];
    let mut cur = String::new();
    for p in pieces {
        if cur.chars().count() + p.chars().count() > max_chars && !cur.trim().is_empty() {
            chunks.push(cur.trim().to_string());
            cur.clear();
        }
        cur.push_str(&p);
    }
    if !cur.trim().is_empty() {
        chunks.push(cur.trim().to_string());
    }
    chunks
}

impl Drop for Formatter {
    fn drop(&mut self) {
        self.unload();
    }
}

/// Linux delivers the parent-death signal when the *thread* that spawned
/// the child exits, not the process. Spawn from one thread that lives as
/// long as the app, so the server dies with VocaFlow and not before.
#[cfg(target_os = "linux")]
fn spawn(cmd: Command) -> std::io::Result<Child> {
    use std::sync::mpsc;
    type Req = (Command, mpsc::Sender<std::io::Result<Child>>);
    static OWNER: once_cell::sync::Lazy<Mutex<mpsc::Sender<Req>>> = once_cell::sync::Lazy::new(|| {
        let (tx, rx) = mpsc::channel::<Req>();
        std::thread::Builder::new()
            .name("vf-child-owner".into())
            .spawn(move || {
                for (mut cmd, reply) in rx {
                    let _ = reply.send(cmd.spawn());
                }
            })
            .expect("child owner thread");
        Mutex::new(tx)
    });
    let (tx, rx) = mpsc::channel();
    OWNER.lock().send((cmd, tx)).map_err(|_| std::io::Error::other("spawner gone"))?;
    rx.recv().map_err(|_| std::io::Error::other("spawner gone"))?
}

#[cfg(not(target_os = "linux"))]
fn spawn(mut cmd: Command) -> std::io::Result<Child> {
    cmd.spawn()
}

fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

#[allow(dead_code)]
fn join_env(var: &str, dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_path_buf()];
    if let Some(old) = std::env::var_os(var) {
        paths.extend(std::env::split_paths(&old));
    }
    std::env::join_paths(paths).unwrap_or_default()
}

/// "ggml_vulkan: 0 = Intel(R) Iris(R) Xe Graphics (Intel Corporation) | uma: 1 …"
/// or Metal's "… GPU name: Apple M2 Pro".
fn gpu_from_log(line: &str) -> Option<String> {
    if let Some(rest) = line.split("ggml_vulkan: 0 = ").nth(1) {
        return Some(rest.split(" (").next().unwrap_or(rest).split(" |").next().unwrap_or(rest).trim().to_string());
    }
    if let Some(rest) = line.split("GPU name:").nth(1) {
        return Some(rest.trim().to_string());
    }
    None
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
    // Our own instructions occasionally get echoed back.
    let leaked = |l: &str| {
        let l = l.trim();
        l == "---"
            || (l.starts_with('(') && l.ends_with(')') && (l.contains("Do not translate") || l.contains("Reply in ")))
            || l.starts_with("App: ")
            || l.starts_with("This is part ")
    };
    t = t.lines().filter(|l| !leaked(l)).collect::<Vec<_>>().join("\n").trim().to_string();
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
    // The input is already rule-cleaned (fillers and self-corrections
    // gone), so a faithful rewrite is about as long as it.
    let min = if kind == Kind::Commit { 0.12 } else { 0.7 };
    if t.is_empty() || ratio < min || ratio > 1.6 + 6.0 / rw {
        log::warn!("formatter output rejected (ratio {ratio:.2})");
        return None;
    }
    if let Some(why) = translated_or_rewritten(raw, &t, kind) {
        log::warn!("formatter output rejected ({why})");
        return None;
    }
    if kind != Kind::Commit {
        if let Some(why) = dropped_sentence(raw, &t) {
            log::warn!("formatter output rejected ({why})");
            return None;
        }
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

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Script {
    Latin,
    Greek,
    Cyrillic,
    Arabic,
    Hebrew,
    Cjk,
    Other,
}

fn script_of(c: char) -> Option<Script> {
    if !c.is_alphabetic() {
        return None;
    }
    Some(match c as u32 {
        0x0041..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F => Script::Cyrillic,
        0x0600..=0x06FF | 0x0750..=0x077F => Script::Arabic,
        0x0590..=0x05FF => Script::Hebrew,
        0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF => Script::Cjk,
        _ => Script::Other,
    })
}

/// The script most letters are written in.
fn dominant_script(s: &str) -> Option<Script> {
    let mut counts: Vec<(Script, usize)> = vec![];
    for sc in s.chars().filter_map(script_of) {
        match counts.iter_mut().find(|(k, _)| *k == sc) {
            Some((_, n)) => *n += 1,
            None => counts.push((sc, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k)
}

fn words_of(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 2).map(str::to_lowercase).collect()
}

/// Guard against the model translating (Greek in, English out) or
/// rewriting instead of cleaning up. Cleanup keeps the speaker's words, so
/// most output words must appear in the input, in the same script.
fn translated_or_rewritten(input: &str, out: &str, kind: Kind) -> Option<String> {
    if let (Some(a), Some(b)) = (dominant_script(input), dominant_script(out)) {
        if a != b {
            return Some(format!("script changed {a:?} -> {b:?}"));
        }
    }
    let seen: std::collections::HashSet<String> = words_of(input).into_iter().collect();
    let ow = words_of(out);
    if ow.len() >= 6 {
        let kept = ow.iter().filter(|w| seen.contains(*w) || w.chars().all(|c| c.is_ascii_digit())).count();
        let share = kept as f32 / ow.len() as f32;
        // Commit summaries legitimately rephrase into the imperative.
        let min = if kind == Kind::Commit { 0.35 } else { 0.55 };
        if share < min {
            return Some(format!("only {:.0}% of words were spoken", share * 100.0));
        }
    }
    None
}

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Small models misspell words in languages they know less well ("Χέρω"
/// for "Ξέρω"). Any output word that isn't in the dictation but is one or
/// two letters off a word that is gets the speaker's word back, with the
/// output's capitalisation.
pub fn restore_words(input: &str, out: &str) -> String {
    let spoken: Vec<Vec<char>> = {
        let mut v: Vec<Vec<char>> = input
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 4)
            .map(|w| w.to_lowercase().chars().collect())
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let mut result = String::with_capacity(out.len());
    let mut word = String::new();
    let flush = |word: &mut String, result: &mut String| {
        if word.chars().count() >= 4 {
            let lower: Vec<char> = word.to_lowercase().chars().collect();
            if !spoken.iter().any(|s| *s == lower) {
                let best = spoken
                    .iter()
                    .filter(|s| (s.len() as isize - lower.len() as isize).abs() <= 1)
                    .map(|s| (edit_distance(s, &lower), s))
                    .filter(|(d, _)| *d >= 1 && *d <= if lower.len() >= 7 { 2 } else { 1 })
                    .min_by_key(|(d, _)| *d);
                if let Some((_, s)) = best {
                    let mut fixed: String = s.iter().collect();
                    if word.chars().next().is_some_and(char::is_uppercase) {
                        let mut c = fixed.chars();
                        fixed = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                    }
                    result.push_str(&fixed);
                    word.clear();
                    return;
                }
            }
        }
        result.push_str(word);
        word.clear();
    };
    for c in out.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut result);
            result.push(c);
        }
    }
    flush(&mut word, &mut result);
    result
}

/// Did the model quietly leave out a sentence? Every spoken sentence with
/// some substance must keep most of its content words (4+ letters).
fn dropped_sentence(input: &str, out: &str) -> Option<String> {
    let content = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 4).map(str::to_lowercase).collect()
    };
    let kept: std::collections::HashSet<String> = content(out).into_iter().collect();
    let mut sentence = String::new();
    let check = |snt: &str| -> Option<String> {
        let words = content(snt);
        if words.len() < 4 {
            return None;
        }
        let share = words.iter().filter(|w| kept.contains(*w)).count() as f32 / words.len() as f32;
        (share < 0.65).then(|| format!("dropped \"{}…\"", snt.trim().chars().take(40).collect::<String>()))
    };
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        sentence.push(c);
        if matches!(c, '.' | '!' | '?' | ';' | '\n' | '\u{37e}') && chars.peek().map_or(true, |n| n.is_whitespace()) {
            if let Some(why) = check(&sentence) {
                return Some(why);
            }
            sentence.clear();
        }
    }
    check(&sentence)
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
    fn rejects_translation() {
        let ctx = Context::default();
        let greek = "Γεια σου Μαρία, ευχαριστώ για το προσχέδιο. Νομίζω ότι πρέπει να μετακινήσουμε την τιμολόγηση μετά το χρονοδιάγραμμα.";
        let english = "Hi Maria, thanks for the draft. I think we should move pricing after the timeline.";
        assert!(accept(greek, english, Kind::Email, &ctx).is_none());
        // Same script, different language (Spanish -> English).
        let es = "Hola Ana, gracias por el borrador. Creo que debemos mover los precios después del calendario.";
        assert!(accept(es, english, Kind::Email, &ctx).is_none());
        // A real cleanup in Greek passes.
        let tidy = "Γεια σου Μαρία,\n\nΕυχαριστώ για το προσχέδιο. Νομίζω ότι πρέπει να μετακινήσουμε την τιμολόγηση μετά το χρονοδιάγραμμα.";
        assert!(accept(greek, tidy, Kind::Email, &ctx).is_some());
    }

    #[test]
    fn strips_echoed_instructions() {
        let ctx = Context::default();
        let input = "The onboarding emails are a launch blocker but nobody owns them yet. Can we get an owner this week?";
        let out = "The onboarding emails are a launch blocker, but nobody owns them yet.\n\n(Reply in English. Do not translate.)\n\nCan we get an owner this week?";
        let got = accept(input, out, Kind::Email, &ctx).unwrap();
        assert!(!got.contains("Do not translate"), "{got}");
    }

    #[test]
    fn restores_misspelled_words() {
        let input = "Ξέρω ότι η ομάδα πωλήσεων θα δυσκολευτεί να εξηγήσει τη διαφορά.";
        let out = "Χέρω ότι η ομάδα πωλήσεων θα δυσκολεύτει να εξηγήσει τη διαφορά.";
        assert_eq!(restore_words(input, out), input);
        // Different words are left alone.
        assert_eq!(restore_words("send it friday", "Send it Friday, please"), "Send it Friday, please");
    }

    #[test]
    fn rejects_dropped_sentences() {
        let ctx = Context::default();
        let input = "Thanks for the plan. I like that we open with the problem statement and the customer quotes, because leadership keeps asking. The timeline section is clear and every milestone has an owner.";
        let dropped = "Thanks for the plan. It sets things up nicely. The timeline section is clear and every milestone has an owner.";
        assert!(accept(input, dropped, Kind::Email, &ctx).is_none());
        let kept = "Thanks for the plan.\n\nI like that we open with the problem statement and the customer quotes, because leadership keeps asking. The timeline section is clear and every milestone has an owner.";
        assert!(accept(input, kept, Kind::Email, &ctx).is_some());
    }

    #[test]
    fn chunks_fit_and_keep_everything() {
        let sentence = "Αυτή είναι μια πρόταση για τη δοκιμή, με αρκετές λέξεις για να μετρήσει. ";
        let text = sentence.repeat(60);
        let total = text.chars().count() / 2;
        let chunks = split_chunks(&text, total, CHUNK_TOKENS);
        assert!(chunks.len() > 1);
        let max_chars = (CHUNK_TOKENS as f64 / total as f64 * text.chars().count() as f64) as usize;
        for c in &chunks {
            assert!(c.chars().count() <= max_chars + sentence.chars().count());
            assert!(c.ends_with('.'));
        }
        let joined: String = chunks.join(" ");
        assert_eq!(words_of(&joined), words_of(&text));
        // Run-on text without punctuation still splits.
        let runon = "λέξη ".repeat(3000);
        let chunks = split_chunks(&runon, 3000, CHUNK_TOKENS);
        assert!(chunks.len() >= 4);
        assert_eq!(chunks.iter().map(|c| c.split_whitespace().count()).sum::<usize>(), 3000);
    }

    #[test]
    fn prompt_never_exceeds_context() {
        // Worst case per request: prefix + the biggest chunk + its rewrite.
        for lang in ["en", "el", "ja"] {
            let prefix = prefix_text(Kind::Email, lang);
            // Characters are an upper bound on tokens for any tokenizer.
            let prefix_tokens = prefix.chars().count();
            let room = N_CTX.saturating_sub(prefix_tokens + CHUNK_TOKENS + 8 + 120 + CTX_MARGIN);
            let max_new = ((CHUNK_TOKENS + 8) * 3 / 2 + 64).min(room);
            assert!(prefix_tokens + CHUNK_TOKENS + 8 + 120 + max_new <= N_CTX - CTX_MARGIN);
            assert!(max_new >= CHUNK_TOKENS, "{lang}: rewrite room {max_new} too small");
        }
    }

    #[test]
    fn drops_repeated_greeting() {
        let ctx = Context { before: "Hi Maria,\n\n".into(), ..Default::default() };
        let out = accept("hi maria thanks for the draft it looks good", "Hi Maria,\n\nThanks for the draft. It looks good.", Kind::Email, &ctx).unwrap();
        assert_eq!(out, "Thanks for the draft. It looks good.");
    }
}
