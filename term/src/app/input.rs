//! Keyboard: our shortcuts first, then everything else goes to the active tab's
//! shell. Paste framing lives here too.
//!
//! | Keys                 | Does                       |
//! |----------------------|----------------------------|
//! | Ctrl+Shift+C         | copy selection             |
//! | Ctrl+V               | paste (apps never see it)  |
//! | Ctrl+Shift+= / - / 0 | font bigger/smaller/reset  |
//! | Ctrl+Shift+T / W     | new tab / close tab        |
//! | Ctrl+Shift+R         | restart in place (new build, tabs kept) |
//!
//! Switching tabs is mouse only (click a tab; middle click closes it).

use super::{App, Error};
use crate::keys;
use crate::xkb::Key;

enum Shortcut {
    Copy,
    Paste,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NewTab,
    CloseTab,
    Restart,
}

fn shortcut(k: &Key) -> Option<Shortcut> {
    let m = k.mods;
    let c = char::from_u32(k.sym)?.to_ascii_lowercase();
    match (m.ctrl, m.shift, m.alt) {
        (true, true, false) => match c {
            'c' => Some(Shortcut::Copy),
            '+' | '=' => Some(Shortcut::ZoomIn),
            '-' | '_' => Some(Shortcut::ZoomOut),
            '0' | ')' => Some(Shortcut::ZoomReset),
            't' => Some(Shortcut::NewTab),
            'w' => Some(Shortcut::CloseTab),
            'r' => Some(Shortcut::Restart),
            _ => None,
        },
        (true, false, false) if c == 'v' => Some(Shortcut::Paste),
        _ => None,
    }
}

impl App {
    /// Returns false when the last tab was closed.
    pub(super) fn key(&mut self, k: &Key) -> Result<bool, Error> {
        if let Some(i) = self.renaming() {
            self.rename_key(i, k)?;
            return Ok(true);
        }
        if let Some(s) = shortcut(k) {
            return self.run_shortcut(s);
        }
        let s = &mut self.tabs[self.active];
        let Some(bytes) = keys::encode(k, s.term.modes.app_cursor) else {
            return Ok(true);
        };
        s.term.reset_view();
        s.pty.write(&bytes);
        if self.mouse.clear_selection() {
            self.redraw = true;
        }
        Ok(true)
    }

    fn run_shortcut(&mut self, s: Shortcut) -> Result<bool, Error> {
        let size = self.renderer.font.size;
        match s {
            Shortcut::Copy => {
                if let Some(text) = self.mouse.selected_text(&self.tabs[self.active].term) {
                    self.win.copy(text)?;
                }
            }
            Shortcut::Paste => self.request_paste()?,
            Shortcut::ZoomIn => self.set_font_size(size + 1.0),
            Shortcut::ZoomOut => self.set_font_size(size - 1.0),
            Shortcut::ZoomReset => self.set_font_size(self.cfg.font_size),
            Shortcut::NewTab => self.open_tab()?,
            Shortcut::CloseTab => return self.close_tab(self.active),
            Shortcut::Restart => self.restart()?,
        }
        Ok(true)
    }

    /// Ask the compositor for the clipboard; the text arrives in `read_paste`.
    pub(super) fn request_paste(&mut self) -> Result<(), Error> {
        if self.paste.is_none()
            && let Some(fd) = self.win.paste()?
        {
            self.paste = Some((fd.into(), Vec::new()));
        }
        Ok(())
    }

    /// Send pasted text: bracketed if the app asked, else newlines become CR.
    pub(super) fn paste_bytes(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        let s = &mut self.tabs[self.active];
        s.term.reset_view();
        if s.term.modes.bracketed_paste {
            // A paste must not be able to end the bracket early. Dropping every ESC is the
            // only safe filter: removing just "ESC[201~" can be dodged by nesting it.
            let clean = text.replace('\x1b', "");
            s.pty.write(b"\x1b[200~");
            s.pty.write(clean.as_bytes());
            s.pty.write(b"\x1b[201~");
        } else {
            let clean = text.replace("\r\n", "\r").replace('\n', "\r");
            s.pty.write(clean.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xkb::Mods;

    fn key(sym: u32, ctrl: bool, shift: bool, alt: bool) -> Key {
        Key {
            sym,
            text: String::new(),
            mods: Mods {
                ctrl,
                shift,
                alt,
                logo: false,
            },
        }
    }

    #[test]
    fn shortcuts() {
        let is = |k: Key, want: fn(&Shortcut) -> bool| shortcut(&k).is_some_and(|s| want(&s));
        assert!(is(key('C' as u32, true, true, false), |s| matches!(
            s,
            Shortcut::Copy
        )));
        assert!(is(key('v' as u32, true, false, false), |s| matches!(
            s,
            Shortcut::Paste
        )));
        assert!(is(key('T' as u32, true, true, false), |s| matches!(
            s,
            Shortcut::NewTab
        )));
        assert!(shortcut(&key(0xff09, true, false, false)).is_none()); // Ctrl+Tab: to the app
        assert!(shortcut(&key('3' as u32, false, false, true)).is_none()); // Alt+3: to the shell
        assert!(shortcut(&key('c' as u32, true, false, false)).is_none()); // Ctrl+C: to the shell
        assert!(shortcut(&key('V' as u32, true, true, false)).is_none());
    }
}
