//! Wayland global shortcut via the XDG GlobalShortcuts portal (GNOME 46+,
//! KDE Plasma 6, Hyprland). The compositor owns the binding and may show
//! its own confirmation the first time; we get press/release signals.

use anyhow::{anyhow, Result};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use parking_lot::Mutex;
use tauri::AppHandle;

const ID: &str = "dictate";
static TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

/// "Alt+D" → "ALT+d" (XDG shortcuts specification).
pub fn to_xdg(accel: &str) -> String {
    accel
        .split('+')
        .map(|p| match p.to_ascii_lowercase().as_str() {
            "alt" | "option" => "ALT".to_string(),
            "ctrl" | "control" | "commandorcontrol" | "cmdorctrl" => "CTRL".to_string(),
            "shift" => "SHIFT".to_string(),
            "super" | "meta" | "cmd" | "command" => "LOGO".to_string(),
            k if k.len() == 1 => k.to_string(),
            _ => p.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Bind asynchronously: the compositor may show a confirmation dialog, and
/// we must not block the caller on it. Failures are logged and surfaced
/// through the "hotkey://error" event.
pub fn bind(app: &AppHandle, accel: &str) -> Result<()> {
    if let Some(t) = TASK.lock().take() {
        t.abort();
    }
    let trigger = to_xdg(accel);
    let app = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        if let Err(e) = listen(&app, &trigger).await {
            log::error!("global shortcuts portal: {e}");
            use tauri::Emitter;
            let _ = app.emit("hotkey://error", "Your desktop blocked the shortcut");
        }
    });
    *TASK.lock() = Some(handle);
    Ok(())
}

async fn listen(app: &AppHandle, trigger: &str) -> Result<()> {
    let portal = GlobalShortcuts::new().await.map_err(|e| anyhow!("{e}"))?;
    let session = portal.create_session().await.map_err(|e| anyhow!("{e}"))?;
    let sc = NewShortcut::new(ID, "Dictate").preferred_trigger(Some(trigger));
    portal.bind_shortcuts(&session, &[sc], None).await.map_err(|e| anyhow!("{e}"))?.response().map_err(|e| anyhow!("{e}"))?;
    let mut on = portal.receive_activated().await.map_err(|e| anyhow!("{e}"))?;
    let mut off = portal.receive_deactivated().await.map_err(|e| anyhow!("{e}"))?;
    loop {
        tokio::select! {
            Some(a) = on.next() => if a.shortcut_id() == ID { crate::hotkey::portal_edge(app, true) },
            Some(d) = off.next() => if d.shortcut_id() == ID { crate::hotkey::portal_edge(app, false) },
            else => break,
        }
    }
    drop(session);
    Ok(())
}
