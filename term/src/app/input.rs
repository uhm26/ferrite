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
//! In a file pane, editing keys (Ctrl+S, Ctrl+Z, ...) live in `view/editor.rs`.

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
            if k.repeat {
                return Ok(true); // holding Ctrl+Shift+W must not close tab after tab
            }
            return self.run_shortcut(s);
        }
        if self.doc_key(k)? {
            return Ok(true);
        }
        let s = self.tabs[self.active].pane_mut();
        let Some(bytes) = keys::encode(k, s.term.modes.app_cursor) else {
            return Ok(true);
        };
        s.term.reset_view();
        s.write(&bytes);
        self.stats.key();
        if self.mouse.clear_selection() {
            self.redraw = true;
        }
        Ok(true)
    }

    fn run_shortcut(&mut self, s: Shortcut) -> Result<bool, Error> {
        let size = self.renderer.font.size;
        match s {
            Shortcut::Copy => {
                let pane = self.tabs[self.active].pane();
                let doc = pane.doc.as_ref().filter(|d| d.mode == crate::view::Mode::Edit);
                let text = doc.map_or_else(|| self.mouse.selected_text(&pane.term), |d| d.buf.selected_text());
                if let Some(text) = text {
                    self.win.copy(text)?;
                }
            }
            // A failed paste or new tab (no fds, fork fails) must not take every tab down.
            Shortcut::Paste => log(self.request_paste()),
            Shortcut::ZoomIn => self.set_font_size(size + 1.0),
            Shortcut::ZoomOut => self.set_font_size(size - 1.0),
            Shortcut::ZoomReset => self.set_font_size(self.cfg.font_size),
            Shortcut::NewTab => log(self.open_tab()),
            Shortcut::CloseTab => return self.close_tab(self.active),
            Shortcut::Restart => self.restart()?,
        }
        Ok(true)
    }

    /// Ask the compositor for the clipboard; the text arrives in `read_paste`.
    /// An image on the clipboard (and no text) is fetched too: it becomes a file and
    /// its path is pasted, which Claude Code attaches as an image (no wl-paste needed).
    pub(super) fn request_paste(&mut self) -> Result<(), Error> {
        let image = self.win.clipboard_is_image();
        if self.paste.is_none()
            && let Some((fd, mime)) = self.win.paste(image)?
        {
            self.paste = Some((fd.into(), Vec::new(), image.then_some(mime)));
        }
        Ok(())
    }

    /// Hand a finished paste to the shell: text as is, an image as a saved file's path.
    pub(super) fn finish_paste(&mut self, data: &[u8], image: Option<String>) {
        let Some(mime) = image else {
            return self.paste_bytes(data);
        };
        match save_image(data, &mime) {
            Ok(path) => self.paste_bytes(path.to_string_lossy().as_bytes()),
            Err(e) => eprintln!("term: cannot save pasted image: {e}"),
        }
    }

    /// Send pasted text: bracketed if the app asked, else newlines become CR.
    pub(super) fn paste_bytes(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        let s = self.tabs[self.active].pane_mut();
        if let Some(d) = s.doc.as_mut() {
            if d.paste(&text) {
                s.refresh();
                self.redraw = true;
            }
            return; // a preview takes no paste
        }
        s.term.reset_view();
        if s.term.modes.bracketed_paste {
            // A paste must not be able to end the bracket early. Dropping every ESC is the
            // only safe filter: removing just "ESC[201~" can be dodged by nesting it.
            let clean = text.replace('\x1b', "");
            s.write(b"\x1b[200~");
            s.write(clean.as_bytes());
            s.write(b"\x1b[201~");
        } else {
            let clean = text.replace("\r\n", "\r").replace('\n', "\r");
            s.write(clean.as_bytes());
        }
    }
}

/// How many pasted images stay on disk (newest first); older ones are removed.
const KEEP_IMAGES: usize = 10;

/// Save pasted image bytes as `$XDG_RUNTIME_DIR/term/paste-<ms>.<ext>` (RAM, gone at
/// reboot), keep the newest [`KEEP_IMAGES`], and return the new file's path.
fn save_image(data: &[u8], mime: &str) -> std::io::Result<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join("term");
    save_in(&dir, data, mime)
}

fn save_in(dir: &std::path::Path, data: &[u8], mime: &str) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    // The mime comes from whatever app owns the clipboard: only a plain word is a name.
    let ext = match mime.strip_prefix("image/") {
        Some("jpeg") => "jpg",
        Some(e) if !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()) => e,
        _ => "png",
    };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join(format!("paste-{ms}.{ext}"));
    std::fs::write(&path, data)?;
    let mut old: Vec<_> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("paste-"))
        })
        .collect();
    old.sort(); // same-length millisecond names sort by time
    let extra = old.len().saturating_sub(KEEP_IMAGES);
    for p in &old[..extra] {
        let _ = std::fs::remove_file(p);
    }
    Ok(path)
}

/// Report an error that only cost one action (a paste, a new tab) and carry on.
pub(super) fn log(r: Result<(), Error>) {
    if let Err(e) = r {
        eprintln!("term: {e}");
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
            repeat: false,
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

    #[test]
    fn pasted_images_keep_the_newest() {
        let dir = std::env::temp_dir().join(format!("term-paste-{}", std::process::id()));
        let mut last = None;
        for _ in 0..12 {
            last = Some(super::save_in(&dir, b"png", "image/jpeg").unwrap());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let last = last.unwrap();
        assert!(last.exists() && last.extension().is_some_and(|e| e == "jpg"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), super::KEEP_IMAGES);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
