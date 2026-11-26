//! Global shortcuts. The dictation key reports both edges (hold vs tap);
//! Escape is only grabbed while listening so VocaFlow never steals it from
//! other apps the rest of the time.

use crate::core::Core;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

static CURRENT: Mutex<Option<Shortcut>> = Mutex::new(None);
static DOWN: Mutex<bool> = Mutex::new(false);

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            let Some(core) = app.try_state::<Arc<Core>>() else { return };
            let core = core.inner().clone();
            if shortcut.matches(tauri_plugin_global_shortcut::Modifiers::empty(), tauri_plugin_global_shortcut::Code::Escape) {
                if event.state == ShortcutState::Pressed {
                    core.cancel();
                }
                return;
            }
            if CURRENT.lock().as_ref() != Some(shortcut) {
                return;
            }
            match event.state {
                ShortcutState::Pressed => {
                    // Key repeat sends a stream of presses; only the first counts.
                    let mut d = DOWN.lock();
                    if *d {
                        return;
                    }
                    *d = true;
                    drop(d);
                    core.on_press();
                }
                ShortcutState::Released => {
                    *DOWN.lock() = false;
                    core.on_release();
                }
            }
        })
        .build()
}

pub fn parse(s: &str) -> Result<Shortcut> {
    s.parse::<Shortcut>().map_err(|e| anyhow!("{e}"))
}

/// Replace the dictation shortcut. Returns an error (and keeps the old
/// one) if the OS refuses it, e.g. another app already owns it.
pub fn register(app: &AppHandle, accel: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    if crate::platform::linux::is_wayland() {
        return crate::platform::portal::bind(app, accel);
    }
    let gs = app.global_shortcut();
    let next = parse(accel)?;
    let prev = CURRENT.lock().take();
    if let Some(p) = &prev {
        let _ = gs.unregister(*p);
    }
    match gs.register(next) {
        Ok(()) => {
            *CURRENT.lock() = Some(next);
            Ok(())
        }
        Err(e) => {
            if let Some(p) = prev {
                if gs.register(p).is_ok() {
                    *CURRENT.lock() = Some(p);
                }
            }
            Err(anyhow!("That shortcut is taken ({e})"))
        }
    }
}

/// Suspend the dictation key while the user records a new one in Settings.
pub fn suspend(app: &AppHandle, on: bool) {
    let gs = app.global_shortcut();
    if let Some(s) = *CURRENT.lock() {
        let _ = if on { gs.unregister(s) } else { gs.register(s) };
    }
}

pub fn set_escape(app: &AppHandle, on: bool) {
    #[cfg(target_os = "linux")]
    if crate::platform::linux::is_wayland() {
        return; // portals can't grab Escape; the hotkey stops instead
    }
    let gs = app.global_shortcut();
    let esc = Shortcut::new(None, tauri_plugin_global_shortcut::Code::Escape);
    if on {
        let _ = gs.register(esc);
    } else {
        let _ = gs.unregister(esc);
    }
}

/// Called by the Wayland portal backend.
#[allow(dead_code)]
pub fn portal_edge(app: &AppHandle, pressed: bool) {
    let Some(core) = app.try_state::<Arc<Core>>() else { return };
    let core = core.inner().clone();
    if pressed {
        let mut d = DOWN.lock();
        if *d {
            return;
        }
        *d = true;
        drop(d);
        core.on_press();
    } else {
        *DOWN.lock() = false;
        core.on_release();
    }
}
