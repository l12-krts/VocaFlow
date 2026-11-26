//! Windows: UI Automation for field context, the system caret for the
//! shimmer position, and overlay window styles.

use crate::context::{Context, Rect};
use uiautomation::patterns::UITextPattern;
use uiautomation::types::TextPatternRangeEndpoint;
use uiautomation::UIAutomation;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, SetWindowLongPtrW,
    SetWindowPos, GUITHREADINFO, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_EX_LAYERED,
};

/// Overlay styles: never activates, never in Alt+Tab, clicks pass through.
pub fn make_overlay(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as _);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let add = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0) as isize;
        SetWindowLongPtrW(h, GWL_EXSTYLE, ex | add);
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn process_name(pid: u32) -> String {
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or("").to_string()
    }
}

fn pretty_app(exe: &str) -> String {
    let base = exe.trim_end_matches(".exe").trim_end_matches(".EXE");
    match base.to_ascii_lowercase().as_str() {
        "chrome" => "Google Chrome".into(),
        "msedge" => "Microsoft Edge".into(),
        "firefox" => "Firefox".into(),
        "olk" | "outlook" => "Outlook".into(),
        "windowsterminal" => "Windows Terminal".into(),
        "code" => "Visual Studio Code".into(),
        "winword" => "Word".into(),
        _ => base.to_string(),
    }
}

fn system_caret(thread: u32) -> Option<Rect> {
    unsafe {
        let mut gti = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(thread, &mut gti).ok()?;
        if gti.hwndCaret.0.is_null() {
            return None;
        }
        let r = gti.rcCaret;
        let mut p = POINT { x: r.left, y: r.top };
        let _ = ClientToScreen(gti.hwndCaret, &mut p);
        let h = (r.bottom - r.top) as f64;
        if h <= 0.0 {
            return None;
        }
        Some(Rect { x: p.x as f64, y: p.y as f64, w: 2.0, h })
    }
}

pub fn capture() -> Context {
    let mut c = Context::default();
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return c;
    }
    let mut pid = 0u32;
    let thread = unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    c.title = window_title(fg);
    c.app_id = process_name(pid);
    c.app = pretty_app(&c.app_id);
    c.caret = system_caret(thread);

    let Ok(ui) = UIAutomation::new() else { return c };
    let Ok(el) = ui.get_focused_element() else { return c };
    c.role = el.get_localized_control_type().ok();
    c.label = [el.get_name().ok(), el.get_help_text().ok()].into_iter().flatten().find(|s| !s.is_empty());

    if let Ok(tp) = el.get_pattern::<UITextPattern>() {
        if let Some(sel) = tp.get_selection().ok().and_then(|v| v.into_iter().next()) {
            if let Ok(before) = tp.get_document_range() {
                if before.move_endpoint_by_range(TextPatternRangeEndpoint::End, &sel, TextPatternRangeEndpoint::Start).is_ok() {
                    let s = before.get_text(-1).unwrap_or_default();
                    let chars: Vec<char> = s.chars().collect();
                    c.before = chars[chars.len().saturating_sub(500)..].iter().collect();
                }
            }
            if let Ok(after) = tp.get_document_range() {
                if after.move_endpoint_by_range(TextPatternRangeEndpoint::Start, &sel, TextPatternRangeEndpoint::Start).is_ok() {
                    c.after = after.get_text(150).unwrap_or_default();
                }
            }
        }
    } else if let Ok(v) = el.get_pattern::<uiautomation::patterns::UIValuePattern>() {
        c.before = v.get_value().unwrap_or_default();
    }
    if c.caret.is_none() {
        if let Ok(r) = el.get_bounding_rectangle() {
            // Unknown caret: shimmer along the field's first line.
            let h = (r.get_bottom() - r.get_top()).min(28) as f64;
            if h > 0.0 {
                c.caret = Some(Rect { x: r.get_left() as f64 + 8.0, y: r.get_top() as f64 + 6.0, w: 2.0, h });
            }
        }
    }
    c
}
