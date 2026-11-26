//! Where is the user typing? App, window title, field role/placeholder and
//! the text around the caret, gathered from the OS accessibility APIs. All
//! of it stays on-device and only feeds the local formatter.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Human-readable app name, e.g. "Mail", "Google Chrome".
    pub app: String,
    /// Stable identifier: bundle id (macOS), exe name (Windows/Linux).
    pub app_id: String,
    pub title: String,
    /// Accessibility role/control type of the focused element.
    pub role: Option<String>,
    /// Placeholder, label or description of the focused field.
    pub label: Option<String>,
    /// Up to ~500 chars before the caret and ~150 after.
    pub before: String,
    pub after: String,
    /// Caret (or field) bounds in screen points, for the paste shimmer.
    pub caret: Option<Rect>,
    /// Monitor work-area containing the focused window, for the pill.
    pub window: Option<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Email,
    Commit,
    Post,
    Chat,
    Code,
    Terminal,
    Search,
    Document,
    Generic,
}

impl Kind {
    /// The word shown in the pill: "Writing <label>…".
    pub fn label(self) -> &'static str {
        match self {
            Kind::Email => "email",
            Kind::Commit => "commit",
            Kind::Post => "post",
            Kind::Chat => "message",
            Kind::Code => "comment",
            Kind::Terminal => "command",
            Kind::Search => "search",
            Kind::Document => "text",
            Kind::Generic => "text",
        }
    }
}

fn has(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Heuristic classification. Cheap and predictable on purpose: the model
/// then gets a clear instruction instead of guessing from raw context.
pub fn classify(c: &Context) -> Kind {
    let app = format!("{} {}", c.app, c.app_id).to_lowercase();
    let title = c.title.to_lowercase();
    let label = c.label.clone().unwrap_or_default().to_lowercase();
    let role = c.role.clone().unwrap_or_default().to_lowercase();
    let before = c.before.to_lowercase();
    let near = format!("{title} {label}");

    // Commit messages: git editors, IDE commit boxes, `git commit -m "`.
    if has(&title, &["commit_editmsg", "commit message", "git commit"])
        || has(&label, &["commit message", "message (ctrl+enter", "message (⌘⏎", "commit summary", "summary (required)"])
        || regex::Regex::new(r#"git commit[^\n]*-m\s*["']?$"#).unwrap().is_match(before.trim_end())
    {
        return Kind::Commit;
    }

    if has(&role, &["search"]) || has(&label, &["search", "find", "filter"]) {
        return Kind::Search;
    }

    // Email clients, and webmail by window title.
    if has(&app, &["mail", "outlook", "thunderbird", "spark", "airmail", "mimestream", "superhuman", "evolution", "geary"])
        || has(&title, &["gmail", "outlook", "proton mail", "fastmail", "inbox", "compose", "new message", "re: ", "fwd: "])
        || has(&label, &["message body", "compose", "to recipients", "subject"])
    {
        return Kind::Email;
    }

    // Social posts.
    if has(&title, &[" / x", "on x:", "twitter", "x.com", "threads", "bluesky", "linkedin", "mastodon"])
        || has(&label, &["what's happening", "what is happening", "post text", "start a post", "what's new"])
    {
        return Kind::Post;
    }

    if has(&app, &["slack", "discord", "teams", "telegram", "whatsapp", "signal", "messages", "messenger", "element", "beeper", "zulip"])
        || has(&title, &["slack", "discord", "whatsapp", "messenger", "teams", "chat"])
        || has(&label, &["message #", "message @", "send a message", "type a message", "write a message", "reply"])
    {
        return Kind::Chat;
    }

    if has(&app, &["terminal", "iterm", "warp", "alacritty", "kitty", "wezterm", "konsole", "gnome-terminal", "ghostty", "windowsterminal", "powershell", "cmd.exe"]) {
        return Kind::Terminal;
    }

    if has(&app, &["code", "cursor", "zed", "intellij", "pycharm", "webstorm", "xcode", "sublime", "nvim", "vim", "android studio", "fleet"]) {
        return Kind::Code;
    }

    if has(&app, &["word", "pages", "docs", "notion", "obsidian", "bear", "notes", "craft", "writer", "libreoffice", "typora", "ulysses"])
        || has(&near, &["google docs", "notion", "document"])
    {
        return Kind::Document;
    }
    Kind::Generic
}

#[cfg(target_os = "macos")]
pub use crate::platform::macos::capture as capture_platform;
#[cfg(target_os = "windows")]
pub use crate::platform::windows::capture as capture_platform;
#[cfg(target_os = "linux")]
pub use crate::platform::linux::capture as capture_platform;

/// The focused window's bounds only: fast enough to run before the pill
/// appears (no accessibility round-trips).
pub fn focused_window() -> Option<Rect> {
    active_win_pos_rs::get_active_window()
        .ok()
        .map(|w| Rect { x: w.position.x, y: w.position.y, w: w.position.width, h: w.position.height })
}

/// Snapshot the focused app and field. Never fails; missing pieces stay empty.
pub fn capture() -> Context {
    let mut c = capture_platform();
    if c.app.is_empty() || c.title.is_empty() {
        if let Ok(w) = active_win_pos_rs::get_active_window() {
            if c.app.is_empty() {
                c.app = w.app_name.clone();
            }
            if c.app_id.is_empty() {
                c.app_id = w.process_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            }
            if c.title.is_empty() {
                c.title = w.title.clone();
            }
            if c.window.is_none() {
                c.window = Some(Rect { x: w.position.x, y: w.position.y, w: w.position.width, h: w.position.height });
            }
        }
    }
    truncate_context(&mut c);
    c
}

fn truncate_context(c: &mut Context) {
    let tail = |s: &str, n: usize| -> String {
        let chars: Vec<char> = s.chars().collect();
        chars[chars.len().saturating_sub(n)..].iter().collect()
    };
    c.before = tail(&c.before, 500);
    c.after = c.after.chars().take(150).collect();
}
