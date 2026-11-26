//! Put text into the focused field: paste via the clipboard (instant, works
//! almost everywhere) and restore whatever was on the clipboard before, or
//! type it for apps the user marked as paste-hostile.

use anyhow::{anyhow, Result};
use std::time::Duration;

enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
    Nothing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Paste,
    Type,
}

pub fn insert(text: &str, method: Method, terminal: bool) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    match method {
        Method::Type => type_text(text),
        Method::Paste => paste(text, terminal),
    }
}

fn paste(text: &str, terminal: bool) -> Result<()> {
    let mut cb = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    let saved = match cb.get_text() {
        Ok(t) => Saved::Text(t),
        Err(_) => cb.get_image().map(|i| Saved::Image(i.to_owned_img())).unwrap_or(Saved::Nothing),
    };
    cb.set_text(text.to_string()).map_err(|e| anyhow!("clipboard: {e}"))?;
    // Let clipboard managers and the target app see the new contents.
    std::thread::sleep(Duration::from_millis(30));
    send_paste_chord(terminal)?;
    // Apps read the clipboard asynchronously after the keystroke; restoring
    // too early pastes the *old* contents in slow apps (Electron, Office).
    std::thread::sleep(Duration::from_millis(350));
    let still_ours = cb.get_text().map(|t| t == text).unwrap_or(false);
    if still_ours {
        let _ = match saved {
            Saved::Text(t) => cb.set_text(t),
            Saved::Image(i) => cb.set_image(i),
            Saved::Nothing => cb.clear(),
        };
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn send_paste_chord(_terminal: bool) -> Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut e = Enigo::new(&Settings::default()).map_err(|e| anyhow!("{e}"))?;
    // kVK_ANSI_V = 9. A raw keycode avoids keyboard-layout lookups that
    // must run on the main thread.
    e.key(Key::Meta, Direction::Press).map_err(|e| anyhow!("{e}"))?;
    let r = e.raw(9, Direction::Click);
    e.key(Key::Meta, Direction::Release).map_err(|e| anyhow!("{e}"))?;
    r.map_err(|e| anyhow!("{e}"))
}

#[cfg(target_os = "windows")]
fn send_paste_chord(_terminal: bool) -> Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut e = Enigo::new(&Settings::default()).map_err(|e| anyhow!("{e}"))?;
    e.key(Key::Control, Direction::Press).map_err(|e| anyhow!("{e}"))?;
    let r = e.key(Key::V, Direction::Click);
    e.key(Key::Control, Direction::Release).map_err(|e| anyhow!("{e}"))?;
    r.map_err(|e| anyhow!("{e}"))
}

#[cfg(target_os = "linux")]
fn send_paste_chord(terminal: bool) -> Result<()> {
    // Terminals on Linux paste with Ctrl+Shift+V.
    if crate::platform::linux::is_wayland() {
        return crate::platform::linux::wayland_paste(terminal);
    }
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut e = Enigo::new(&Settings::default()).map_err(|e| anyhow!("{e}"))?;
    e.key(Key::Control, Direction::Press).map_err(|e| anyhow!("{e}"))?;
    if terminal {
        e.key(Key::Shift, Direction::Press).map_err(|e| anyhow!("{e}"))?;
    }
    let r = e.key(Key::Unicode('v'), Direction::Click);
    if terminal {
        let _ = e.key(Key::Shift, Direction::Release);
    }
    e.key(Key::Control, Direction::Release).map_err(|e| anyhow!("{e}"))?;
    r.map_err(|e| anyhow!("{e}"))
}

fn type_text(text: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    if crate::platform::linux::is_wayland() {
        return crate::platform::linux::wayland_type(text);
    }
    use enigo::{Enigo, Keyboard, Settings};
    let mut e = Enigo::new(&Settings::default()).map_err(|e| anyhow!("{e}"))?;
    // Type line by line; some apps drop characters on very long bursts.
    // Newlines are Shift+Return so chat apps insert a line instead of sending.
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            use enigo::{Direction, Key};
            e.key(Key::Shift, Direction::Press).map_err(|e| anyhow!("{e}"))?;
            let r = e.key(Key::Return, Direction::Click);
            e.key(Key::Shift, Direction::Release).map_err(|e| anyhow!("{e}"))?;
            r.map_err(|e| anyhow!("{e}"))?;
        }
        if !line.is_empty() {
            e.text(line).map_err(|e| anyhow!("{e}"))?;
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}
