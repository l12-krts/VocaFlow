use crate::audio;
use crate::core::Core;
use crate::history::Item;
use crate::models::ModelsState;
use crate::settings::Settings;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

type Res<T> = Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct Platform {
    os: &'static str,
    wayland: bool,
}

#[tauri::command]
pub fn platform() -> Platform {
    Platform {
        os: if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        },
        #[cfg(target_os = "linux")]
        wayland: crate::platform::linux::is_wayland(),
        #[cfg(not(target_os = "linux"))]
        wayland: false,
    }
}

#[tauri::command]
pub fn get_settings(core: State<Arc<Core>>) -> Settings {
    core.settings()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, core: State<Arc<Core>>, patch: serde_json::Value) -> Res<Settings> {
    let before = core.settings();
    // Validate the shortcut with the OS before saving it.
    if let Some(hk) = patch.get("hotkey").and_then(|v| v.as_str()) {
        if hk != before.hotkey {
            crate::hotkey::register(&app, hk).map_err(err)?;
        }
    }
    let after = core.settings.lock().patch(patch).map_err(err)?;
    if after.quality != before.quality {
        core.quality_changed();
    }
    if after.formatting != before.formatting {
        core.sync_models();
    }
    if after.launch_at_login != before.launch_at_login {
        crate::sync_autostart(&app, after.launch_at_login);
    }
    let _ = app.emit("settings://changed", &after);
    Ok(after)
}

#[tauri::command]
pub fn models_state(core: State<Arc<Core>>) -> ModelsState {
    core.models.state()
}

#[tauri::command]
pub fn retry_download(core: State<Arc<Core>>) {
    core.sync_models();
}

#[tauri::command]
pub fn list_mics() -> Vec<audio::MicDevice> {
    audio::list_devices()
}

#[tauri::command]
pub fn mic_meter(core: State<Arc<Core>>, on: bool) {
    core.meter(on);
}

#[tauri::command]
pub fn history(core: State<Arc<Core>>) -> Vec<Item> {
    core.history.lock().items.clone()
}

#[tauri::command]
pub fn clear_history(app: AppHandle, core: State<Arc<Core>>) {
    let mut h = core.history.lock();
    h.clear();
    let _ = app.emit("history://changed", &h.items);
}

#[tauri::command]
pub fn delete_history(app: AppHandle, core: State<Arc<Core>>, id: String) {
    let mut h = core.history.lock();
    h.delete(&id);
    let _ = app.emit("history://changed", &h.items);
}

#[tauri::command]
pub fn copy_text(text: String) -> Res<()> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(err)
}

#[derive(Serialize)]
pub struct Permissions {
    microphone: &'static str,
    accessibility: &'static str,
}

#[tauri::command]
pub fn permissions() -> Permissions {
    #[cfg(target_os = "macos")]
    {
        Permissions {
            microphone: crate::platform::macos::mic_status(),
            accessibility: if crate::platform::macos::ax_trusted(false) { "granted" } else { "denied" },
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Permissions { microphone: "granted", accessibility: "granted" }
    }
}

#[tauri::command]
pub fn request_permission(app: AppHandle, kind: String) {
    #[cfg(target_os = "macos")]
    {
        match kind.as_str() {
            "microphone" => crate::platform::macos::request_mic(),
            _ => {
                crate::platform::macos::ax_trusted(true);
            }
        }
    }
    let _ = (app, kind);
}

#[tauri::command]
pub fn capture_hotkey(app: AppHandle, on: bool) {
    crate::hotkey::suspend(&app, on);
}

#[tauri::command]
pub fn pill_ready() {}

#[tauri::command]
pub fn pill_shape(app: AppHandle, width: f64, visible: bool, reduced: Option<bool>) {
    #[cfg(target_os = "macos")]
    {
        let reduced = reduced.unwrap_or(false);
        let _ = app.run_on_main_thread(move || unsafe { crate::platform::macos::shape_glass(width, visible, reduced) });
    }
    let _ = (app, width, visible, reduced);
}

/// The Settings page has painted: now it can be shown without a flash.
#[tauri::command]
pub fn main_ready(app: AppHandle, dark: bool) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(move || unsafe { crate::platform::macos::set_glass_dark(dark) });
    }
    let _ = (&app, dark);
}

#[tauri::command]
pub fn hide_main(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}
