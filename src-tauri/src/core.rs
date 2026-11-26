//! The dictation controller: hotkey edges in, text out.
//!
//!  press ──► listening ──(release after hold)──────────► transcribing ─► formatting ─► insert
//!               │  └──(quick tap: hands-free) ─(press)──┘
//!               └──(Esc)──► cancelled
//!
//! Recording starts on the key-down edge in both modes, so the first word
//! is never lost while we decide whether this is a hold or a tap.

use crate::audio::{self, Capture};
use crate::cleanup;
use crate::context::{self, Context, Kind};
use crate::formatter::{self, Formatter};
use crate::history::{History, Item};
use crate::insert::{self, Method};
use crate::models::{self, Models, Status};
use crate::settings::{Settings, Store};
use crate::stt::{self, Engine};
use crate::ui;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// Held longer than this = push-to-talk; shorter = hands-free toggle.
const HOLD_THRESHOLD: Duration = Duration::from_millis(300);
/// Hands-free stops itself after this much silence (like macOS Dictation).
const SILENCE_STOP: Duration = Duration::from_secs(30);
const MAX_RECORDING: Duration = Duration::from_secs(10 * 60);

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Listening,
    Transcribing,
    Formatting,
    Error,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PillState {
    pub phase: Phase,
    pub hands_free: bool,
    pub format: Option<String>,
    pub message: Option<String>,
}

struct Session {
    capture: Capture,
    pressed_at: Instant,
    hands_free: bool,
    ctx: Context,
}

pub struct Core {
    pub app: AppHandle,
    pub settings: Mutex<Store>,
    pub models: Arc<Models>,
    pub history: Mutex<History>,
    stt: Engine,
    formatter: Formatter,
    session: Mutex<Option<Session>>,
    phase: Arc<Mutex<Phase>>,
    meter: Mutex<Option<Capture>>,
    /// Bumped on every new state so delayed "back to idle" never clobbers
    /// a newer dictation.
    epoch: Arc<Mutex<u64>>,
}

impl Core {
    pub fn new(app: AppHandle, config_dir: std::path::PathBuf, data_dir: std::path::PathBuf, llama_dir: std::path::PathBuf) -> Arc<Self> {
        let formatter = Formatter::new();
        formatter.configure(llama_dir, data_dir.join("formatter.log"));
        Arc::new(Self {
            settings: Mutex::new(Store::load(config_dir.clone())),
            models: Models::new(data_dir.join("models")),
            history: Mutex::new(History::load(data_dir)),
            stt: Engine::new(),
            formatter,
            session: Mutex::new(None),
            phase: Arc::new(Mutex::new(Phase::Idle)),
            meter: Mutex::new(None),
            epoch: Arc::new(Mutex::new(0)),
            app,
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().value.clone()
    }

    fn phase_handle(&self) -> (Arc<Mutex<Phase>>, Arc<Mutex<u64>>) {
        (self.phase.clone(), self.epoch.clone())
    }

    fn emit_pill(&self, phase: Phase, hands_free: bool, format: Option<&str>, message: Option<&str>) {
        *self.phase.lock() = phase;
        *self.epoch.lock() += 1;
        if phase == Phase::Idle && cfg!(target_os = "windows") {
            // Park the overlays once the close animation has played.
            let app = self.app.clone();
            let epoch = *self.epoch.lock();
            let me_phase = self.phase_handle();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1200));
                if me_phase.0.lock().eq(&Phase::Idle) && *me_phase.1.lock() == epoch {
                    ui::park(&app, "pill");
                    ui::park(&app, "fx");
                }
            });
        }
        let _ = self.app.emit_to(
            "pill",
            "pill://state",
            PillState { phase, hands_free, format: format.map(str::to_string), message: message.map(str::to_string) },
        );
    }

    /// Show a short message in the pill, then fade back to idle.
    fn flash(self: &Arc<Self>, message: &str, ms: u64) {
        self.emit_pill(Phase::Error, false, None, Some(message));
        let me = self.clone();
        let epoch = *self.epoch.lock();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            if *me.epoch.lock() == epoch {
                me.emit_pill(Phase::Idle, false, None, None);
            }
        });
    }

    pub fn emit_models(&self) {
        let _ = self.app.emit("models://state", self.models.state());
    }

    // ---------- Hotkey edges -------------------------------------------

    pub fn on_press(self: &Arc<Self>) {
        let phase = *self.phase.lock();
        match phase {
            Phase::Idle | Phase::Error => self.start(),
            Phase::Listening => {
                // Second press in hands-free mode ends the dictation.
                let hands_free = self.session.lock().as_ref().is_some_and(|s| s.hands_free);
                if hands_free {
                    self.stop();
                }
            }
            // Busy: ignore. A fresh dictation starts once text is inserted.
            Phase::Transcribing | Phase::Formatting => {}
        }
    }

    pub fn on_release(self: &Arc<Self>) {
        let mut g = self.session.lock();
        let Some(s) = g.as_mut() else { return };
        if s.hands_free {
            return;
        }
        if s.pressed_at.elapsed() >= HOLD_THRESHOLD {
            drop(g);
            self.stop();
        } else {
            s.hands_free = true;
            drop(g);
            self.emit_pill(Phase::Listening, true, None, None);
        }
    }

    pub fn cancel(self: &Arc<Self>) {
        if let Some(s) = self.session.lock().take() {
            s.capture.cancel();
            crate::hotkey::set_escape(&self.app, false);
            self.flash("Cancelled", 900);
        }
    }

    fn start(self: &Arc<Self>) {
        // Hold-vs-tap is timed from the key press itself.
        let pressed_at = Instant::now();
        let st = self.settings();
        // Make sure there's a model before opening the mic.
        if self.models.usable_speech(st.quality).is_none() {
            let msg = match self.models.speech.lock().clone() {
                Status::Downloading { received, total } if total > 0 => format!("Downloading model {}%", received * 100 / total),
                Status::Error { .. } => "Model download failed".to_string(),
                _ => "Preparing model".to_string(),
            };
            self.flash(&msg, 1800);
            return;
        }
        // The meter in Settings owns the mic; release it.
        if let Some(m) = self.meter.lock().take() {
            m.cancel();
        }
        // The pill must appear the instant the key goes down, so only the
        // cheap lookup (which screen?) happens before it; reading the field
        // through the accessibility APIs runs in parallel below.
        ui::place_pill(&self.app, context::focused_window());
        let capture = match Capture::start(st.mic_id.as_deref(), true) {
            Ok(c) => c,
            Err(e) => {
                log::error!("mic: {e:#}");
                self.flash("Microphone unavailable", 2000);
                return;
            }
        };
        *self.session.lock() = Some(Session { capture, pressed_at, hands_free: false, ctx: Context::default() });
        self.emit_pill(Phase::Listening, false, None, None);
        log::info!("listening after {:?}", pressed_at.elapsed());
        crate::hotkey::set_escape(&self.app, true);
        self.spawn_level_pump();

        let me = self.clone();
        std::thread::spawn(move || {
            let ctx = context::capture();
            log::debug!("context: app={:?} title={:?} role={:?}", ctx.app, ctx.title, ctx.role);
            if let Some(s) = me.session.lock().as_mut() {
                if s.pressed_at == pressed_at {
                    s.ctx = ctx;
                }
            }
        });
    }

    /// Stream mic level to the pill (~30 fps) and watch for silence in
    /// hands-free mode.
    fn spawn_level_pump(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(33));
            let (level, stop) = {
                let g = me.session.lock();
                let Some(s) = g.as_ref() else { break };
                let silent = s.hands_free && s.capture.since_voice() > SILENCE_STOP;
                (s.capture.level(), silent || s.capture.elapsed() > MAX_RECORDING)
            };
            let _ = me.app.emit_to("pill", "audio://level", level);
            if stop {
                me.stop();
                break;
            }
        });
    }

    fn stop(self: &Arc<Self>) {
        let Some(s) = self.session.lock().take() else { return };
        crate::hotkey::set_escape(&self.app, false);
        let started = s.pressed_at;
        self.emit_pill(Phase::Transcribing, false, None, None);
        let me = self.clone();
        std::thread::spawn(move || {
            let samples = s.capture.finish();
            me.process(samples, s.ctx, started);
        });
    }

    // ---------- Pipeline ---------------------------------------------

    fn process(self: &Arc<Self>, samples: Vec<f32>, start_ctx: Context, started: Instant) {
        let st = self.settings();
        let Some(audio) = audio::prepare(samples) else {
            self.flash("Didn't catch that", 1400);
            return;
        };
        let Some((tier, path)) = self.models.usable_speech(st.quality) else {
            self.flash("Model not ready", 1600);
            return;
        };
        let req = stt::Request { language: &st.language, preferred: &st.preferred_languages, dictionary: &st.dictionary, quality: tier };
        let t0 = Instant::now();
        let transcript = match self.stt.transcribe(&path, &audio, &req) {
            Ok(t) => t,
            Err(e) => {
                log::error!("stt: {e:#}");
                self.flash("Couldn't transcribe", 1800);
                return;
            }
        };
        log::info!("stt {:?} ({} samples, lang {})", t0.elapsed(), audio.len(), transcript.language);
        if transcript.text.trim().is_empty() {
            self.flash("Didn't catch that", 1400);
            return;
        }

        // Where is the text going *now*? The user may have moved focus
        // while speaking; the start snapshot is the fallback.
        let mut ctx = context::capture();
        if ctx.app.is_empty() {
            ctx = start_ctx;
        }
        let kind = context::classify(&ctx);
        let raw = transcript.text;
        let mut text = cleanup::basic(&raw, kind);

        // Loaded and warmed in the background by `sync_models`; never load
        // a 1 GB model in the middle of a dictation.
        let formatter_ready = st.formatting && self.formatter.is_loaded();
        if st.formatting && !formatter_ready && self.models.has_formatter() {
            // The server exited (driver reset, killed): bring it back for
            // next time; this dictation uses the rules.
            let me = self.clone();
            std::thread::spawn(move || {
                if let Err(e) = me.formatter.warm(&me.models.path(models::FORMATTER[0].file)) {
                    log::error!("restart formatter: {e:#}");
                }
            });
        }
        if formatter_ready && formatter::worth_formatting(&raw, kind) {
            self.emit_pill(Phase::Formatting, false, Some(kind.label()), None);
            let fmt = || -> anyhow::Result<Option<String>> {
                self.formatter.format(&text, kind, &ctx, Duration::from_secs(10))
            };
            match fmt() {
                Ok(Some(t)) => text = t,
                Ok(None) => {}
                Err(e) => log::error!("formatter: {e:#}"),
            }
        }

        let text = cleanup::join_with_field(&text, &ctx);
        let method = if st.typing_apps.iter().any(|a| a.eq_ignore_ascii_case(&ctx.app_id) || a.eq_ignore_ascii_case(&ctx.app)) {
            Method::Type
        } else {
            Method::Paste
        };

        // Shimmer first so it lands with the text, not after it.
        let sweep = ui::place_fx(&self.app, ctx.caret, text.trim().chars().count());
        if let Some((w, h)) = sweep {
            let _ = self.app.emit_to("fx", "fx://insert", serde_json::json!({ "width": w, "height": h }));
        }
        // Without Accessibility, macOS silently drops synthetic keystrokes:
        // keep the text on the clipboard and say so instead of losing it.
        #[cfg(target_os = "macos")]
        if !crate::platform::macos::ax_trusted(false) {
            let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.trim().to_string()));
            self.record(&text, &raw, &ctx, kind, started);
            self.flash("Copied, allow Accessibility", 2400);
            return;
        }
        if let Err(e) = insert::insert(&text, method, kind == Kind::Terminal) {
            log::error!("insert: {e:#}");
            self.record(&text, &raw, &ctx, kind, started);
            self.flash("Copied to clipboard", 1800);
            let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.trim().to_string()));
            return;
        }
        self.record(&text, &raw, &ctx, kind, started);
        self.emit_pill(Phase::Idle, false, None, None);
    }

    fn record(&self, text: &str, raw: &str, ctx: &Context, kind: Kind, started: Instant) {
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let item = Item {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.trim().to_string(),
            raw: raw.to_string(),
            app: (!ctx.app.is_empty()).then(|| ctx.app.clone()),
            format: (kind != Kind::Generic).then(|| kind.label().to_string()),
            at,
            duration_ms: started.elapsed().as_millis() as u64,
        };
        let mut h = self.history.lock();
        h.push(item);
        let _ = self.app.emit("history://changed", &h.items);
    }

    // ---------- Models -------------------------------------------------

    /// Bring models in line with settings: download the selected tier,
    /// fetch or drop the formatter, warm what's ready.
    pub fn sync_models(self: &Arc<Self>) {
        let st = self.settings();
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let m2 = me.clone();
            me.models.ensure_speech(st.quality, move |s| { let _ = m2.app.emit("models://state", s); }).await;
            if let Some((_, p)) = me.models.usable_speech(st.quality) {
                let me2 = me.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    if let Err(e) = me2.stt.warm(&p) {
                        log::error!("warm stt: {e:#}");
                    }
                })
                .await;
            }
        });
        let me = self.clone();
        if st.formatting {
            tauri::async_runtime::spawn(async move {
                let m2 = me.clone();
                me.models.ensure_formatter(move |s| { let _ = m2.app.emit("models://state", s); }).await;
                if me.models.has_formatter() {
                    let me2 = me.clone();
                    let _ = tauri::async_runtime::spawn_blocking(move || {
                        let r = me2.formatter.warm(&me2.models.path(models::FORMATTER[0].file));
                        if let Err(e) = r {
                            log::error!("warm formatter: {e:#}");
                        }
                    })
                    .await;
                }
            });
        } else {
            self.formatter.unload();
            self.models.pause_formatter();
            self.emit_models();
        }
    }

    /// Stop child processes before the app exits.
    pub fn shutdown(&self) {
        self.formatter.unload();
    }

    /// Accelerator in use, for Settings ("Intel(R) Iris(R) Xe Graphics").
    pub fn gpu(&self) -> Option<String> {
        stt::gpu_name().or_else(|| self.formatter.gpu.lock().clone())
    }

    pub fn quality_changed(self: &Arc<Self>) {
        // Keep the old model loaded until the new one is on disk; sync
        // swaps it once the download completes.
        self.sync_models();
    }

    // ---------- Settings meter -----------------------------------------

    pub fn meter(self: &Arc<Self>, on: bool) {
        let mut g = self.meter.lock();
        if let Some(m) = g.take() {
            m.cancel();
        }
        if !on || self.session.lock().is_some() {
            return;
        }
        let mic = self.settings().mic_id;
        match Capture::start(mic.as_deref(), false) {
            Ok(c) => {
                *g = Some(c);
                drop(g);
                let me = self.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(33));
                    let lv = match me.meter.lock().as_ref() {
                        Some(c) => c.level(),
                        None => break,
                    };
                    if let Some(w) = me.app.get_webview_window("main") {
                        let _ = w.emit("audio://level", lv);
                    }
                });
            }
            Err(e) => log::warn!("meter: {e:#}"),
        }
    }
}
