//! The location bar: the path as text to edit or paste over. It opens on Ctrl+L or a
//! click on empty space in the path bar (whole path selected, so typing or pasting
//! replaces it), or by typing `/` or `~`. Return goes there (a file: its folder, with
//! the file selected); Esc or a click elsewhere puts the crumbs back.

use std::path::{Path, PathBuf};

use frame::ui::{Rect, Ui};

use super::sidebar::home;
use crate::browse::Browser;

const NAME: &str = "location";

#[derive(Default)]
pub struct Location {
    pub open: bool,
    text: String,
    /// Ctrl+V: the app should read the clipboard and hand it to [`Location::paste`].
    pub paste_wanted: bool,
    /// Ctrl+C: text the app should put on the clipboard.
    pub copy: Option<String>,
    /// Clipboard text that arrived, applied on the next draw.
    pasted: Option<String>,
}

impl Location {
    /// Open the bar: the current path, all selected (`with` None), or starting with
    /// `with` (a typed `/` or `~`).
    pub fn start(&mut self, ui: &mut Ui, b: &Browser, with: Option<&str>) {
        self.open = true;
        match with {
            None => {
                self.text = b.cwd.display().to_string();
                ui.take_focus_all(NAME);
            }
            Some(t) => {
                self.text = t.to_string();
                ui.take_focus(NAME);
            }
        }
    }

    pub fn paste(&mut self, text: String) {
        self.pasted = Some(text);
    }

    /// The field in `r`. Closes on Return (after going there), Esc or focus loss.
    pub fn draw(&mut self, ui: &mut Ui, b: &mut Browser, r: Rect) {
        // An empty clipboard pastes nothing (it must not wipe the selected path).
        if let Some(t) = self.pasted.take().filter(|t| !t.trim().is_empty()) {
            ui.paste_into(&mut self.text, &t);
        }
        ui.field(NAME, r, &mut self.text, "Type a path");
        if ui.submitted(NAME) {
            self.open = false;
            go(b, &resolve(&b.cwd, &self.text));
        } else if !ui.focused(NAME) {
            self.open = false;
        }
        self.keys(ui);
    }

    /// Ctrl keys the field leaves over: V paste, C copy, X cut (the selection, or
    /// the whole path when nothing is selected for copy).
    fn keys(&mut self, ui: &mut Ui) {
        for k in std::mem::take(&mut ui.s.input.keys) {
            match (k.mods.ctrl, k.sym) {
                (true, 0x76) => self.paste_wanted = true,
                (true, 0x63) => {
                    let sel = ui.field_selection(&self.text);
                    let text = if sel.is_empty() { &self.text } else { sel };
                    self.copy = Some(text.to_string());
                }
                (true, 0x78) => self.copy = Some(ui.cut(&mut self.text)).filter(|t| !t.is_empty()),
                _ => {}
            }
        }
    }
}

/// Show `p`: a folder opens; a file opens its folder with the file selected.
fn go(b: &mut Browser, p: &Path) {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => b.go(p.to_path_buf(), None),
        Ok(_) => match (p.parent(), p.file_name()) {
            // A file in the folder shown: just select it.
            (Some(parent), Some(name)) if parent == b.cwd => {
                if let Some(i) = b.entries.iter().position(|e| e.os == name) {
                    b.sel = i;
                }
            }
            (Some(parent), name) => b.go(parent.to_path_buf(), name),
            _ => {}
        },
        Err(e) => b.error = Some(format!("{}: {e}", p.display())),
    }
}

/// What the typed text means: `~` is home, a relative path starts at `cwd`.
fn resolve(cwd: &Path, text: &str) -> PathBuf {
    let t = text.trim();
    let p = match t.strip_prefix('~') {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(t),
    };
    if p.is_absolute() { p } else { cwd.join(p) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ctrl+L, End, Shift+Home, type /usr, Return: the browser goes to /usr.
    #[test]
    fn edit_and_return_goes_there() {
        use frame::ui::{State, Text, theme::FAMILY};
        use frame::window::Canvas;
        use frame::xkb::{Key, Mods};
        let key = |sym: u32, text: &str, shift: bool| Key {
            sym,
            text: text.into(),
            mods: Mods {
                shift,
                ..Mods::default()
            },
            repeat: false,
        };
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut b = Browser::new(PathBuf::from("/tmp"));
        let mut loc = Location::default();
        let mut px = vec![0u32; 400 * 40];
        let r = Rect::new(0.0, 0.0, 400.0, 32.0);
        let mut frame =
            |s: &mut State, keys: Vec<Key>, loc: &mut Location, b: &mut Browser, open: bool| {
                s.input.keys = keys;
                let mut ui = s.begin(
                    Canvas {
                        px: &mut px,
                        w: 400,
                        h: 40,
                    },
                    0.0,
                );
                if open {
                    loc.start(&mut ui, b, None);
                }
                loc.draw(&mut ui, b, r);
                ui.end();
            };
        frame(&mut s, vec![], &mut loc, &mut b, true);
        // End, Shift+Home (select all), then type "/usr" over it.
        let keys = [(0xff57, "", false), (0xff50, "", true), (0x2f, "/", false)]
            .into_iter()
            .chain([(0x75, "u", false), (0x73, "s", false), (0x72, "r", false)])
            .map(|(s, t, sh)| key(s, t, sh))
            .collect();
        frame(&mut s, keys, &mut loc, &mut b, false);
        frame(
            &mut s,
            vec![key(0xff0d, "\r", false)],
            &mut loc,
            &mut b,
            false,
        );
        assert_eq!(b.cwd, PathBuf::from("/usr"));
        assert!(!loc.open);
    }

    #[test]
    fn typed_paths_resolve() {
        let cwd = Path::new("/usr");
        assert_eq!(resolve(cwd, " /etc "), PathBuf::from("/etc"));
        assert_eq!(resolve(cwd, "lib"), PathBuf::from("/usr/lib"));
        assert_eq!(resolve(cwd, "~"), home());
        assert_eq!(resolve(cwd, "~/projects"), home().join("projects"));
    }
}
