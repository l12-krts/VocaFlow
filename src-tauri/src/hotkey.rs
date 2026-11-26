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
/// Why the saved shortcut couldn't be registered at launch, for Settings.
pub static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static DOWN: Mutex<bool> = Mutex::new(false);

enum Edge {
    Press,
    Release,
    Cancel,
}

/// Hotkey edges are handled on one worker thread, in order. The OS
/// callback runs on the UI event loop, which must never wait on the
/// microphone or the accessibility APIs.
static WORKER: once_cell::sync::Lazy<std::sync::mpsc::Sender<(Arc<Core>, Edge)>> = once_cell::sync::Lazy::new(|| {
    let (tx, rx) = std::sync::mpsc::channel::<(Arc<Core>, Edge)>();
    std::thread::Builder::new()
        .name("vf-hotkey".into())
        .spawn(move || {
            for (core, edge) in rx {
                match edge {
                    Edge::Press => core.on_press(),
                    Edge::Release => core.on_release(),
                    Edge::Cancel => core.cancel(),
                }
            }
        })
        .expect("hotkey worker");
    tx
});

fn dispatch(core: Arc<Core>, edge: Edge) {
    let _ = WORKER.send((core, edge));
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            let Some(core) = app.try_state::<Arc<Core>>() else { return };
            let core = core.inner().clone();
            if shortcut.matches(tauri_plugin_global_shortcut::Modifiers::empty(), tauri_plugin_global_shortcut::Code::Escape) {
                if event.state == ShortcutState::Pressed {
                    dispatch(core, Edge::Cancel);
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
                    log::debug!("hotkey down");
                    #[cfg(target_os = "windows")]
                    if shortcut.mods.contains(tauri_plugin_global_shortcut::Modifiers::ALT) {
                        crate::platform::windows::mask_alt_menu();
                    }
                    dispatch(core, Edge::Press);
                }
                ShortcutState::Released => {
                    *DOWN.lock() = false;
                    log::debug!("hotkey up");
                    dispatch(core, Edge::Release);
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
            log::info!("dictation shortcut: {accel}");
            *CURRENT.lock() = Some(next);
            *LAST_ERROR.lock() = None;
            Ok(())
        }
        Err(e) => {
            *LAST_ERROR.lock() = Some("Taken by another app".into());
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
        dispatch(core, Edge::Press);
    } else {
        *DOWN.lock() = false;
        dispatch(core, Edge::Release);
    }
}
