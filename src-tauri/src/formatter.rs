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
        Self { server: Mutex::new(None), dir: Mutex::new(None), log: Mutex::new(None), gpu: Arc::new(Mutex::new(None)) }
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
            .args(["-c", "2048", "-np", "1", "-t", &threads.to_string()])
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
        // email is as fast as the tenth.
        let _ = self.complete(&prefix_text(Kind::Email), 1, Duration::from_secs(30));
        Ok(())
    }

    fn complete(&self, prompt: &str, n_predict: usize, budget: Duration) -> Result<Option<String>> {
        let (port, key) = {
            let g = self.server.lock();
            let s = g.as_ref().ok_or_else(|| anyhow!("formatter not running"))?;
            (s.port, s.key.clone())
        };
        let client = reqwest::blocking::Client::builder().no_proxy().timeout(budget + Duration::from_secs(2)).build()?;
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
        // Cut short by the time limit: don't paste half a sentence.
        if v.get("stop_type").and_then(|s| s.as_str()) == Some("limit") || v.get("stopped_limit").and_then(|b| b.as_bool()) == Some(true) {
            if v.get("timings").and_then(|t| t.get("predicted_ms")).and_then(|m| m.as_f64()).unwrap_or(0.0) >= budget.as_millis() as f64 * 0.95 {
                return Ok(None);
            }
        }
        Ok(v.get("content").and_then(|c| c.as_str()).map(str::to_string))
    }

    /// Format `raw` for the field described by `ctx`. Returns None if the
    /// model is unavailable, too slow, or its output failed the checks.
    pub fn format(&self, raw: &str, kind: Kind, ctx: &Context, budget: Duration) -> Result<Option<String>> {
        let started = Instant::now();
        let prompt = format!("{}{}", prefix_text(kind), suffix_text(raw, ctx));
        let words = raw.split_whitespace().count();
        let max_new = (words * 3 + 48).min(700);
        let Some(text) = self.complete(&prompt, max_new, budget)? else {
            log::warn!("formatter over budget, using rules");
            return Ok(None);
        };
        log::info!("formatter: {:?}", started.elapsed());
        Ok(accept(raw, &text, kind, ctx))
    }
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
