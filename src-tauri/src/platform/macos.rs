//! macOS: Liquid Glass behind the pill, Accessibility (AX) for context.
//!
//! The pill window is transparent; a native glass view sits beneath the
//! webview and is animated by Core Animation to the exact rect the CSS pill
//! occupies, on the same curve and duration, so refraction and tint move as
//! one object. macOS 26+ gets NSGlassEffectView (real Liquid Glass); older
//! systems get an NSVisualEffectView HUD material.

#![allow(non_upper_case_globals)]

use crate::context::{Context, Rect};
use accessibility_sys::*;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::ffi::c_void;
use std::ptr;

// ---------- Glass ---------------------------------------------------------

pub const PILL_H: f64 = 72.0;
/// Pill top inside the window (pill.css `padding-top`), measured from the
/// top. AppKit's origin is bottom-left, so the glass uses the window height.
pub const PILL_TOP: f64 = 26.0;
pub const WINDOW_H: f64 = 112.0;
const PILL_BOTTOM: f64 = WINDOW_H - PILL_TOP - PILL_H;

struct Glass {
    view: Retained<AnyObject>,
    window_w: f64,
    shown: bool,
    width: f64,
}

static mut GLASS: Option<Glass> = None;

#[allow(static_mut_refs)]
fn glass() -> Option<&'static mut Glass> {
    unsafe { GLASS.as_mut() }
}

/// Install the glass view behind the webview. Main thread only.
pub unsafe fn install_glass(ns_window: *mut c_void, window_w: f64) {
    let window = ns_window as *mut AnyObject;
    let content: *mut AnyObject = msg_send![window, contentView];
    if content.is_null() {
        return;
    }
    let frame = NSRect::new(NSPoint::new(window_w / 2.0, PILL_BOTTOM), NSSize::new(0.0, PILL_H));

    let view: Retained<AnyObject> = if let Some(cls) = AnyClass::get(c"NSGlassEffectView") {
        let v: *mut AnyObject = msg_send![cls, alloc];
        let v: *mut AnyObject = msg_send![v, initWithFrame: frame];
        let _: () = msg_send![v, setCornerRadius: PILL_H / 2.0];
        Retained::from_raw(v).unwrap()
    } else {
        let cls = AnyClass::get(c"NSVisualEffectView").unwrap();
        let v: *mut AnyObject = msg_send![cls, alloc];
        let v: *mut AnyObject = msg_send![v, initWithFrame: frame];
        // NSVisualEffectMaterialHUDWindow = 13, BlendingModeBehindWindow = 0, StateActive = 1
        let _: () = msg_send![v, setMaterial: 13isize];
        let _: () = msg_send![v, setBlendingMode: 0isize];
        let _: () = msg_send![v, setState: 1isize];
        let _: () = msg_send![v, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![v, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: PILL_H / 2.0];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
        Retained::from_raw(v).unwrap()
    };
    let _: () = msg_send![&*view, setAlphaValue: 0.0f64];
    // NSWindowBelow = -1: under the WKWebView, which stays transparent.
    let none: *mut AnyObject = ptr::null_mut();
    let _: () = msg_send![content, addSubview: &*view, positioned: -1isize, relativeTo: none];
    GLASS = Some(Glass { view, window_w, shown: false, width: 0.0 });
}

fn rect_for(window_w: f64, w: f64, scale: f64, dy: f64) -> NSRect {
    let sw = w * scale;
    let sh = PILL_H * scale;
    NSRect::new(NSPoint::new((window_w - sw) / 2.0, PILL_BOTTOM + (PILL_H - sh) / 2.0 - dy), NSSize::new(sw, sh))
}

unsafe fn animate(duration: f64, curve: [f32; 4], f: impl FnOnce()) {
    let ctx_cls = AnyClass::get(c"NSAnimationContext").unwrap();
    let _: () = msg_send![ctx_cls, beginGrouping];
    let ctx: *mut AnyObject = msg_send![ctx_cls, currentContext];
    let _: () = msg_send![ctx, setDuration: duration];
    let _: () = msg_send![ctx, setAllowsImplicitAnimation: true];
    let tf = objc2_quartz_core::CAMediaTimingFunction::functionWithControlPoints(curve[0], curve[1], curve[2], curve[3]);
    let _: () = msg_send![ctx, setTimingFunction: &*tf];
    f();
    let _: () = msg_send![ctx_cls, endGrouping];
}

const EASE_OUT: [f32; 4] = [0.23, 1.0, 0.32, 1.0];
const DRAWER: [f32; 4] = [0.32, 0.72, 0.0, 1.0];

/// Mirror the CSS pill (Pill.tsx): open as a circle that stretches to its
/// width, morph between states, close by collapsing back into a circle.
pub unsafe fn shape_glass(width: f64, visible: bool, reduced: bool) {
    let Some(g) = glass() else { return };
    let v = &*g.view;
    let proxy: *mut AnyObject = msg_send![v, animator];
    match (g.shown, visible) {
        (false, true) => {
            let from = if reduced { rect_for(g.window_w, width, 1.0, 0.0) } else { circle(g.window_w, 0.8, -10.0) };
            let _: () = msg_send![v, setFrame: from];
            animate(if reduced { 0.12 } else { 0.16 }, EASE_OUT, || {
                let _: () = msg_send![proxy, setAlphaValue: 1.0f64];
            });
            animate(if reduced { 0.0 } else { 0.5 }, DRAWER, || {
                let _: () = msg_send![proxy, setFrame: rect_for(g.window_w, width, 1.0, 0.0)];
            });
        }
        (true, false) => {
            animate(if reduced { 0.12 } else { 0.36 }, DRAWER, || {
                if !reduced {
                    let _: () = msg_send![proxy, setFrame: circle(g.window_w, 0.82, -6.0)];
                }
                let _: () = msg_send![proxy, setAlphaValue: 0.0f64];
            });
        }
        (true, true) if (width - g.width).abs() > 0.5 => {
            animate(if reduced { 0.0 } else { 0.42 }, DRAWER, || {
                let _: () = msg_send![proxy, setFrame: rect_for(g.window_w, width, 1.0, 0.0)];
            });
        }
        _ => {}
    }
    g.shown = visible;
    if visible {
        g.width = width;
    }
}

/// The closed shape: a PILL_H circle, scaled, centred where the pill sits.
fn circle(window_w: f64, scale: f64, dy: f64) -> NSRect {
    let d = PILL_H * scale;
    NSRect::new(NSPoint::new((window_w - d) / 2.0, PILL_BOTTOM + (PILL_H - d) / 2.0 - dy), NSSize::new(d, d))
}

pub unsafe fn set_glass_dark(dark: bool) {
    let Some(g) = glass() else { return };
    // Let the glass follow the app theme rather than the system one.
    let appearance_cls = AnyClass::get(c"NSAppearance").unwrap();
    let name = objc2_foundation::NSString::from_str(if dark { "NSAppearanceNameDarkAqua" } else { "NSAppearanceNameAqua" });
    let ap: *mut AnyObject = msg_send![appearance_cls, appearanceNamed: &*name];
    let _: () = msg_send![&*g.view, setAppearance: ap];
}

/// Accessory policy: no Dock icon, no app switcher entry.
pub unsafe fn set_accessory() {
    let cls = AnyClass::get(c"NSApplication").unwrap();
    let app: *mut AnyObject = msg_send![cls, sharedApplication];
    let _: bool = msg_send![app, setActivationPolicy: 1isize];
}

/// Pill must float over full-screen apps and every Space.
pub unsafe fn make_overlay(ns_window: *mut c_void) {
    let w = ns_window as *mut AnyObject;
    // canJoinAllSpaces | fullScreenAuxiliary | stationary | ignoresCycle
    let behavior: usize = (1 << 0) | (1 << 8) | (1 << 4) | (1 << 6);
    let _: () = msg_send![w, setCollectionBehavior: behavior];
    let _: () = msg_send![w, setHasShadow: false];
    // NSStatusWindowLevel = 25
    let _: () = msg_send![w, setLevel: 25isize];
    let _ = sel!(setIgnoresMouseEvents:);
}

// ---------- Accessibility ----------------------------------------------

pub fn ax_trusted(prompt: bool) -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::from(prompt).as_CFType())]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef() as _)
    }
}

unsafe fn attr(el: AXUIElementRef, name: &str) -> Option<CFType> {
    let mut out: core_foundation::base::CFTypeRef = ptr::null();
    let n = CFString::new(name);
    if AXUIElementCopyAttributeValue(el, n.as_concrete_TypeRef(), &mut out) == kAXErrorSuccess && !out.is_null() {
        Some(CFType::wrap_under_create_rule(out))
    } else {
        None
    }
}

unsafe fn attr_str(el: AXUIElementRef, name: &str) -> Option<String> {
    attr(el, name)?.downcast::<CFString>().map(|s| s.to_string()).filter(|s| !s.is_empty())
}

unsafe fn param_attr(el: AXUIElementRef, name: &str, param: core_foundation::base::CFTypeRef) -> Option<CFType> {
    let mut out: core_foundation::base::CFTypeRef = ptr::null();
    let n = CFString::new(name);
    if AXUIElementCopyParameterizedAttributeValue(el, n.as_concrete_TypeRef(), param, &mut out) == kAXErrorSuccess && !out.is_null() {
        Some(CFType::wrap_under_create_rule(out))
    } else {
        None
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CFRange {
    location: isize,
    length: isize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

unsafe fn range_value(r: CFRange) -> CFType {
    let v = AXValueCreate(kAXValueTypeCFRange, &r as *const _ as *const c_void);
    CFType::wrap_under_create_rule(v as _)
}

unsafe fn frontmost() -> (String, String, i32) {
    let ws_cls = AnyClass::get(c"NSWorkspace").unwrap();
    let ws: *mut AnyObject = msg_send![ws_cls, sharedWorkspace];
    let app: *mut AnyObject = msg_send![ws, frontmostApplication];
    if app.is_null() {
        return Default::default();
    }
    let name: *mut objc2_foundation::NSString = msg_send![app, localizedName];
    let bid: *mut objc2_foundation::NSString = msg_send![app, bundleIdentifier];
    let pid: i32 = msg_send![app, processIdentifier];
    let s = |p: *mut objc2_foundation::NSString| if p.is_null() { String::new() } else { (*p).to_string() };
    (s(name), s(bid), pid)
}

pub fn capture() -> Context {
    unsafe {
        let mut c = Context::default();
        let (app, bid, pid) = frontmost();
        c.app = app;
        c.app_id = bid;
        if pid <= 0 || !ax_trusted(false) {
            return c;
        }
        let app_el = AXUIElementCreateApplication(pid);
        if app_el.is_null() {
            return c;
        }
        // Chromium and Electron only build their AX tree when asked.
        let manual = CFString::new("AXManualAccessibility");
        AXUIElementSetAttributeValue(app_el, manual.as_concrete_TypeRef(), CFBoolean::true_value().as_CFTypeRef());
        AXUIElementSetMessagingTimeout(app_el, 0.25);

        if let Some(win) = attr(app_el, "AXFocusedWindow") {
            let w = win.as_CFTypeRef() as AXUIElementRef;
            c.title = attr_str(w, "AXTitle").unwrap_or_default();
        }
        let Some(focused) = attr(app_el, "AXFocusedUIElement") else {
            core_foundation::base::CFRelease(app_el as _);
            return c;
        };
        let el = focused.as_CFTypeRef() as AXUIElementRef;
        c.role = attr_str(el, "AXSubrole").or_else(|| attr_str(el, "AXRole"));
        c.label = attr_str(el, "AXPlaceholderValue")
            .or_else(|| attr_str(el, "AXDescription"))
            .or_else(|| attr_str(el, "AXTitle"))
            .or_else(|| attr_str(el, "AXHelp"));

        // Caret position from the (zero-length) selection.
        let mut sel = CFRange::default();
        let have_sel = attr(el, "AXSelectedTextRange")
            .map(|v| AXValueGetValue(v.as_CFTypeRef() as AXValueRef, kAXValueTypeCFRange, &mut sel as *mut _ as *mut c_void))
            .unwrap_or(false);
        if have_sel {
            let count = attr(el, "AXNumberOfCharacters")
                .and_then(|v| v.downcast::<core_foundation::number::CFNumber>())
                .and_then(|n| n.to_i64())
                .unwrap_or(0) as isize;
            let caret = sel.location.max(0);
            let start = (caret - 500).max(0);
            if let Some(s) = param_attr(el, "AXStringForRange", range_value(CFRange { location: start, length: caret - start }).as_CFTypeRef()) {
                c.before = s.downcast::<CFString>().map(|s| s.to_string()).unwrap_or_default();
            }
            let after_len = (count - caret).clamp(0, 150);
            if after_len > 0 {
                if let Some(s) = param_attr(el, "AXStringForRange", range_value(CFRange { location: caret, length: after_len }).as_CFTypeRef()) {
                    c.after = s.downcast::<CFString>().map(|s| s.to_string()).unwrap_or_default();
                }
            }
            // Bounds of the character before the caret (a zero-length range
            // returns nothing in many apps).
            let probe = if caret > 0 { CFRange { location: caret - 1, length: 1 } } else { CFRange { location: 0, length: 1 } };
            if let Some(b) = param_attr(el, "AXBoundsForRange", range_value(probe).as_CFTypeRef()) {
                let mut r = CGRect::default();
                if AXValueGetValue(b.as_CFTypeRef() as AXValueRef, kAXValueTypeCGRect, &mut r as *mut _ as *mut c_void) && r.h > 0.0 {
                    let x = if caret > 0 { r.x + r.w } else { r.x };
                    c.caret = Some(Rect { x, y: r.y, w: 2.0, h: r.h });
                }
            }
        } else if let Some(v) = attr_str(el, "AXValue") {
            // No selection API: treat the whole value as "before".
            c.before = v;
        }
        core_foundation::base::CFRelease(app_el as _);
        c
    }
}

// ---------- Microphone permission ---------------------------------------

pub fn mic_status() -> &'static str {
    unsafe {
        let Some(cls) = AnyClass::get(c"AVCaptureDevice") else { return "unknown" };
        let media = objc2_foundation::NSString::from_str("soun");
        // AVAuthorizationStatus: 0 notDetermined, 1 restricted, 2 denied, 3 authorized
        let s: isize = msg_send![cls, authorizationStatusForMediaType: &*media];
        match s {
            3 => "granted",
            0 => "unknown",
            _ => "denied",
        }
    }
}

pub fn request_mic() {
    // Opening the device triggers the system prompt; nothing to record.
    std::thread::spawn(|| {
        if let Ok(c) = crate::audio::Capture::start(None, false) {
            std::thread::sleep(std::time::Duration::from_millis(300));
            c.cancel();
        }
    });
}
