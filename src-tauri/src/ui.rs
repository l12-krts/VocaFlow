//! Windows: Settings ("main"), the pill overlay and the paste-shimmer overlay.
//!
//! The two overlays are created once, shown once at launch and never hidden
//! again: they are transparent and click-through, and their content
//! animates in and out in CSS. Showing or hiding a native window can steal
//! focus from the app the user is typing into, which is the one thing a
//! dictation tool must never do.

use crate::context::Rect;
use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Set at launch: show Settings as soon as its page reports it has painted.
pub static SHOW_WHEN_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub const PILL_W: f64 = 360.0;
pub const PILL_H: f64 = 112.0;

pub fn create_main(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window("main") {
        return Ok(w);
    }
    #[allow(unused_mut)]
    let mut b = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("VocaFlow")
        .inner_size(780.0, 560.0)
        .min_inner_size(680.0, 480.0)
        .resizable(true)
        .visible(false)
        .center();
    #[cfg(target_os = "macos")]
    {
        b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    }
    let w = b.build()?;
    // Closing Settings hides it; VocaFlow keeps running in the tray.
    let w2 = w.clone();
    w.on_window_event(move |e| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = e {
            api.prevent_close();
            let _ = w2.hide();
            // The level meter's mic must not outlive the window.
            if let Some(core) = w2.app_handle().try_state::<std::sync::Arc<crate::core::Core>>() {
                core.meter(false);
            }
        }
    });
    let _ = visible;
    Ok(w)
}

pub fn show_main(app: &AppHandle, route: Option<&str>) {
    let Ok(w) = create_main(app, true) else { return };
    if let Some(r) = route {
        use tauri::Emitter;
        let _ = w.emit("main://navigate", r);
    }
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

fn overlay(app: &AppHandle, label: &str, page: &str, w: f64, h: f64) -> tauri::Result<WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title("VocaFlow")
        .inner_size(w, h)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()?;
    // Click-through is applied after the first show: on Linux the GDK
    // window doesn't exist until the window is mapped.
    Ok(win)
}

pub fn create_overlays(app: &AppHandle) -> tauri::Result<()> {
    let pill = overlay(app, "pill", "pill.html", PILL_W, PILL_H)?;
    let fx = overlay(app, "fx", "fx.html", 600.0, 80.0)?;
    #[cfg(target_os = "macos")]
    {
        let p = pill.clone();
        let f = fx.clone();
        app.run_on_main_thread(move || unsafe {
            if let Ok(ns) = p.ns_window() {
                crate::platform::macos::make_overlay(ns);
                crate::platform::macos::install_glass(ns, PILL_W);
            }
            if let Ok(ns) = f.ns_window() {
                crate::platform::macos::make_overlay(ns);
            }
        })?;
    }
    place_pill(app, None);
    let _ = pill.show();
    let _ = fx.show();
    #[cfg(not(target_os = "windows"))]
    {
        // Clicks pass straight through to the apps underneath.
        let _ = pill.set_ignore_cursor_events(true);
        let _ = fx.set_ignore_cursor_events(true);
    }
    #[cfg(target_os = "windows")]
    {
        // See platform::windows::make_overlay: no layered click-through on
        // Windows; idle overlays take clicks on one pixel only.
        for w in [&pill, &fx] {
            if let Ok(h) = w.hwnd() {
                crate::platform::windows::make_overlay(h.0 as isize);
            }
        }
        set_active(app, "pill", false);
        set_active(app, "fx", false);
    }
    Ok(())
}

/// Windows only: whether an overlay takes clicks (only while it shows
/// something). macOS and Linux overlays are always click-through.
pub fn set_active(app: &AppHandle, label: &str, active: bool) {
    #[cfg(target_os = "windows")]
    if let Some(w) = app.get_webview_window(label) {
        if let Ok(h) = w.hwnd() {
            crate::platform::windows::set_hit_area(h.0 as isize, active);
        }
    }
    let _ = (app, label, active);
}

#[cfg(target_os = "windows")]
fn raise(w: &WebviewWindow) {
    if let Ok(h) = w.hwnd() {
        crate::platform::windows::raise(h.0 as isize);
    }
}

/// Bottom-centre of the screen the user is working on, just above the
/// taskbar/Dock. `anchor` is the focused window (physical pixels on
/// Windows/Linux, points on macOS); falls back to the mouse.
pub fn place_pill(app: &AppHandle, anchor: Option<Rect>) {
    let Some(pill) = app.get_webview_window("pill") else { return };
    let monitor = anchor
        .and_then(|r| {
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            #[cfg(target_os = "macos")]
            let (cx, cy) = {
                let s = pill.scale_factor().unwrap_or(1.0);
                (cx * s, cy * s)
            };
            app.monitor_from_point(cx, cy).ok().flatten()
        })
        .or_else(|| app.cursor_position().ok().and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten()))
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let wa = m.work_area();
    let size = PhysicalSize::new((PILL_W * scale) as u32, (PILL_H * scale) as u32);
    let x = wa.position.x + (wa.size.width as i32 - size.width as i32) / 2;
    let y = wa.position.y + wa.size.height as i32 - size.height as i32;
    let _ = pill.set_size(LogicalSize::new(PILL_W, PILL_H));
    let _ = pill.set_position(PhysicalPosition::new(x, y));
    #[cfg(target_os = "windows")]
    {
        set_active(app, "pill", true);
        raise(&pill);
    }
}

/// Move the shimmer overlay over the line where text is being inserted,
/// starting at the caret captured just before the paste. Returns the sweep
/// size in CSS pixels, or None when the caret is unknown.
pub fn place_fx(app: &AppHandle, caret: Option<Rect>, chars: usize) -> Option<(f64, f64)> {
    let fx = app.get_webview_window("fx")?;
    let c = caret?;
    // Rough width of what was inserted, capped to a sensible line.
    let line_h = c.h.clamp(12.0, 64.0);
    let width = (chars as f64 * line_h * 0.48).clamp(80.0, 720.0);
    let pad = 24.0;
    #[cfg(target_os = "macos")]
    {
        let _ = fx.set_size(LogicalSize::new(width + pad * 2.0, line_h + pad * 2.0));
        let _ = fx.set_position(tauri::LogicalPosition::new(c.x - pad, c.y - pad));
        return Some((width, line_h));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let scale = app.monitor_from_point(c.x, c.y).ok().flatten().map(|m| m.scale_factor()).unwrap_or(1.0);
        // Caret rects are physical pixels here; the font scales with DPI.
        let _ = fx.set_size(PhysicalSize::new((width + pad * 2.0 * scale) as u32, (line_h + pad * 2.0 * scale) as u32));
        let _ = fx.set_position(PhysicalPosition::new((c.x - pad * scale) as i32, (c.y - pad * scale) as i32));
        #[cfg(target_os = "windows")]
        {
            set_active(app, "fx", true);
            raise(&fx);
        }
        Some((width / scale, line_h / scale))
    }
}
