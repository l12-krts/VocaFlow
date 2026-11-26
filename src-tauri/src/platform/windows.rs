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

/// Overlay window setup.
///
/// Tauri's transparent windows on Windows render through DirectComposition
/// without a redirection bitmap. Making them layered (which is how
/// click-through works) leaves them blank after the first repaint and makes
/// DWM fall back to the legacy frame: the "Windows 7" outline. So overlays
/// are never layered. They have no frame, border, corners or transitions,
/// never take focus, and VocaFlow parks them off-screen while idle so they
/// can't intercept clicks.
pub fn make_overlay(hwnd: isize) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY,
        DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, SWP_FRAMECHANGED, SWP_NOZORDER, WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
        WS_POPUP, WS_SYSMENU, WS_THICKFRAME, WS_EX_WINDOWEDGE, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE,
    };
    unsafe {
        let h = HWND(hwnd as _);
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        let frame = (WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_BORDER.0 | WS_DLGFRAME.0) as isize;
        SetWindowLongPtrW(h, GWL_STYLE, (style & !frame) | WS_POPUP.0 as isize);

        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let drop = (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_WINDOWEDGE.0 | WS_EX_CLIENTEDGE.0 | WS_EX_DLGMODALFRAME.0 | WS_EX_STATICEDGE.0) as isize;
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex & !drop) | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize);

        let set = |attr, value: u32| {
            let _ = DwmSetWindowAttribute(h, attr, &value as *const u32 as *const core::ffi::c_void, 4);
        };
        set(DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED.0 as u32);
        set(DWMWA_TRANSITIONS_FORCEDISABLED, 1);
        set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND.0 as u32);
        set(DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
        let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        raise(hwnd);
    }
}

/// Topmost again without activating (the taskbar can end up above us).
pub fn raise(hwnd: isize) {
    unsafe {
        let _ = SetWindowPos(HWND(hwnd as _), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
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

/// Tie a child process to VocaFlow: when the app exits or crashes, the OS
/// closes the job and kills the child with it.
pub fn kill_with_parent(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    static JOB: once_cell::sync::OnceCell<isize> = once_cell::sync::OnceCell::new();
    let job = *JOB.get_or_init(|| unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else { return 0 };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job.0 as isize
    });
    if job != 0 {
        unsafe {
            let _ = AssignProcessToJobObject(HANDLE(job as _), HANDLE(child.as_raw_handle() as _));
        }
    }
}

/// With an Alt-based shortcut, Windows sees Alt pressed and released with
/// nothing in between (the hotkey swallows the letter) and many apps open
/// their menu bar, which then eats our paste. Tapping an unassigned virtual
/// key while Alt is still down cancels that (AutoHotkey's "menu mask").
pub fn mask_alt_menu() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY};
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0xE8), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 },
        },
    };
    unsafe {
        SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32);
    }
}
