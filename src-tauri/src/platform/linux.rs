//! Linux: AT-SPI for field context, X11 via enigo, Wayland via wtype /
//! ydotool for synthetic input (Wayland forbids it for normal clients).

use crate::context::{Context, Rect};
use anyhow::{anyhow, Result};
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::text::TextProxy;
use atspi::{AccessibilityConnection, CoordType, State};
use futures_util::StreamExt;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::process::Command;
use std::time::Duration;

pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false) || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn which(bin: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {bin}")).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn wayland_paste(terminal: bool) -> Result<()> {
    if which("wtype") {
        let mut c = Command::new("wtype");
        c.args(["-M", "ctrl"]);
        if terminal {
            c.args(["-M", "shift"]);
        }
        c.args(["-k", "v"]);
        if terminal {
            c.args(["-m", "shift"]);
        }
        c.args(["-m", "ctrl"]);
        if c.status().map(|s| s.success()).unwrap_or(false) {
            return Ok(());
        }
    }
    if which("ydotool") {
        // Linux input event codes: 29 LEFTCTRL, 42 LEFTSHIFT, 47 V.
        let seq: &[&str] = if terminal { &["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"] } else { &["29:1", "47:1", "47:0", "29:0"] };
        if Command::new("ydotool").arg("key").args(seq).status().map(|s| s.success()).unwrap_or(false) {
            return Ok(());
        }
    }
    // XWayland apps still accept X11 synthetic input.
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut e = Enigo::new(&Settings::default()).map_err(|e| anyhow!("{e}"))?;
    e.key(Key::Control, Direction::Press).map_err(|e| anyhow!("{e}"))?;
    let r = e.key(Key::Unicode('v'), Direction::Click);
    e.key(Key::Control, Direction::Release).map_err(|e| anyhow!("{e}"))?;
    r.map_err(|e| anyhow!("Paste needs wtype or ydotool on Wayland ({e})"))
}

pub fn wayland_type(text: &str) -> Result<()> {
    if which("wtype") && Command::new("wtype").arg("--").arg(text).status().map(|s| s.success()).unwrap_or(false) {
        return Ok(());
    }
    if which("ydotool") && Command::new("ydotool").args(["type", "--"]).arg(text).status().map(|s| s.success()).unwrap_or(false) {
        return Ok(());
    }
    Err(anyhow!("Typing needs wtype or ydotool on Wayland"))
}

/// Whether synthetic input will work for native Wayland apps.
pub fn wayland_input_ready() -> bool {
    !is_wayland() || which("wtype") || which("ydotool")
}

// ---------- AT-SPI focus tracking ---------------------------------------

struct Tracker {
    conn: AccessibilityConnection,
    focused: Mutex<Option<atspi::ObjectRef>>,
}

static TRACKER: OnceCell<&'static Tracker> = OnceCell::new();

/// Start listening for focus changes. AT-SPI has no "what is focused now"
/// call, so we remember the last object that gained focus.
pub fn start_focus_tracking() {
    tauri::async_runtime::spawn(async {
        // Ask toolkits to expose accessibility (GTK/Qt only do so when
        // something is listening).
        let _ = atspi::connection::set_session_accessibility(true).await;
        let conn = match AccessibilityConnection::new().await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("AT-SPI unavailable: {e}");
                return;
            }
        };
        if let Err(e) = conn.register_event::<atspi::events::object::StateChangedEvent>().await {
            log::warn!("AT-SPI focus events unavailable: {e}");
            return;
        }
        // Caret moves fire on every keystroke in a text field: this catches
        // the field even if it was focused before VocaFlow started.
        let _ = conn.register_event::<atspi::events::object::TextCaretMovedEvent>().await;
        let tracker: &'static Tracker = Box::leak(Box::new(Tracker { conn, focused: Mutex::new(None) }));
        let _ = TRACKER.set(tracker);
        let mut events = tracker.conn.event_stream();
        while let Some(ev) = events.next().await {
            match ev {
                Ok(atspi::Event::Object(atspi::events::ObjectEvents::StateChanged(e))) if e.state == State::Focused && e.enabled => {
                    *tracker.focused.lock() = Some(e.item.clone());
                }
                Ok(atspi::Event::Object(atspi::events::ObjectEvents::TextCaretMoved(e))) => {
                    *tracker.focused.lock() = Some(e.item.clone());
                }
                _ => {}
            }
        }
    });
}

pub fn capture() -> Context {
    let Some(t) = TRACKER.get().copied() else { return Context::default() };
    let Some(obj) = t.focused.lock().clone() else { return Context::default() };
    let fut = async move {
        let conn = t.conn.connection();
        let acc: AccessibleProxy = obj.as_accessible_proxy(conn).await.ok()?;
        let mut c = Context::default();
        c.role = acc.get_role_name().await.ok();
        let name = acc.name().await.unwrap_or_default();
        let desc = acc.description().await.unwrap_or_default();
        let ph = acc.get_attributes().await.ok().and_then(|a| a.get("placeholder-text").cloned());
        c.label = [name, desc, ph.unwrap_or_default()].into_iter().find(|s| !s.is_empty());
        if let Ok(app) = acc.get_application().await {
            if let Ok(app) = app.as_accessible_proxy(conn).await {
                c.app = app.name().await.unwrap_or_default();
                c.app_id = c.app.to_lowercase();
            }
        }
        if let Ok(text) = TextProxy::builder(conn).destination(obj.name.clone()).ok()?.path(obj.path.clone()).ok()?.build().await {
            if let (Ok(caret), Ok(count)) = (text.caret_offset().await, text.character_count().await) {
                if caret >= 0 {
                    c.before = text.get_text((caret - 500).max(0), caret).await.unwrap_or_default();
                    c.after = text.get_text(caret, (caret + 150).min(count)).await.unwrap_or_default();
                    let probe = if caret > 0 { caret - 1 } else { caret };
                    if let Ok((x, y, w, h)) = text.get_character_extents(probe, CoordType::Screen).await {
                        if h > 0 {
                            c.caret = Some(Rect { x: (x + if caret > 0 { w } else { 0 }) as f64, y: y as f64, w: 2.0, h: h as f64 });
                        }
                    }
                }
            }
        }
        Some(c)
    };
    tauri::async_runtime::block_on(async { tokio::time::timeout(Duration::from_millis(250), fut).await.ok().flatten() }).unwrap_or_default()
}
