//! The keyboard, Nemo's way: arrows, Home/End and PgUp/PgDn move; Return opens;
//! Backspace or Alt+↑ go up; Alt+←/→ history; Alt+Home home. Typing a name jumps to
//! it (type-to-find, shown in the status bar; Esc clears). Shortcuts: Ctrl+C/X/V
//! copy, cut, paste files; F2 rename; Delete to Trash; Ctrl+Shift+N new folder;
//! Ctrl+Shift+C copy path; Ctrl+D bookmark; Ctrl+H hidden files; F4 terminal here;
//! Ctrl+Shift+F search subfolders; Ctrl+± / Ctrl+0 zoom the grid; tabs in tabs.rs;
//! Shift+arrows extend the selection, Ctrl+A selects all; Ctrl+L or `/` `~`
//! edit the path; Ctrl+F filter; Menu or Shift+F10 the context menu; Ctrl+Q quits.

use frame::ui::Ui;
use frame::xkb::Key;

use super::actions::Action;
use super::{LIST, View, home};
use crate::browse::Browser;
use crate::ops::trash;

const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_HOME: u32 = 0xff50;
const KEY_LEFT: u32 = 0xff51;
const KEY_UP: u32 = 0xff52;
const KEY_RIGHT: u32 = 0xff53;
const KEY_DOWN: u32 = 0xff54;
pub(super) const KEY_PAGE_UP: u32 = 0xff55;
pub(super) const KEY_PAGE_DOWN: u32 = 0xff56;
const KEY_END: u32 = 0xff57;
const KEY_MENU: u32 = 0xff67;
const KEY_F2: u32 = 0xffbf;
const KEY_F4: u32 = 0xffc1;
const KEY_F10: u32 = 0xffc7;
const KEY_DELETE: u32 = 0xffff;
const KEY_PLUS: u32 = 0x2b;
const KEY_EQUAL: u32 = 0x3d;
const KEY_MINUS: u32 = 0x2d;
const KEY_0: u32 = 0x30;
const KEY_KP_ADD: u32 = 0xffab;
const KEY_KP_SUBTRACT: u32 = 0xffad;

/// The letter of a Ctrl shortcut, any case (Shift makes the keysym upper-case).
pub(super) fn ctrl_letter(k: &Key) -> Option<u8> {
    let c = u8::try_from(k.sym).ok()?;
    (k.mods.ctrl && c.is_ascii_alphabetic()).then(|| c.to_ascii_lowercase())
}

/// The action a key stands for.
fn shortcut(k: &Key) -> Option<Action> {
    match (k.sym, ctrl_letter(k), k.mods.shift) {
        (KEY_F2, ..) => Some(Action::Rename),
        (KEY_F4, ..) => Some(Action::Terminal),
        (KEY_DELETE, ..) => Some(Action::Trash),
        (_, Some(b'n'), false) if k.mods.alt => Some(Action::NewFile(0)),
        (_, Some(b'x'), false) => Some(Action::Cut),
        (_, Some(b'c'), false) => Some(Action::Copy),
        (_, Some(b'v'), false) => Some(Action::Paste),
        (_, Some(b'n'), true) => Some(Action::NewFolder),
        (_, Some(b'c'), true) => Some(Action::CopyPath),
        (_, Some(b'd'), false) => Some(Action::Bookmark),
        (_, Some(b'h'), false) => Some(Action::Hidden),
        _ => None,
    }
}

impl View {
    pub(super) fn keys(&mut self, ui: &mut Ui, b: &mut Browser) {
        if self.keyboard_taken() || self.pick_typing(ui) {
            return; // a field or the menu has the keyboard
        }
        let mut keys = std::mem::take(&mut ui.s.input.keys).into_iter();
        while let Some(k) = keys.next() {
            let handled = self.command(ui, b, &k) || self.motion(b, &k) || self.typed(ui, b, &k);
            if self.loc.open {
                // Keys typed after `/` in the same frame belong to the field.
                ui.s.input.keys = keys.collect();
                return;
            }
            if handled {
                self.scroll.follow();
            }
        }
    }

    /// Shortcuts, bars and the menu. True if `k` was one.
    fn command(&mut self, ui: &mut Ui, b: &mut Browser, k: &Key) -> bool {
        if let Some(a) = shortcut(k) {
            let in_trash = b.cwd == trash::files_dir();
            let a = if a == Action::Trash && in_trash {
                Action::Erase
            } else {
                a
            };
            // Paste and the folder-wide actions don't need an entry.
            let on_item = !matches!(
                a,
                Action::Paste
                    | Action::NewFolder
                    | Action::NewFile(_)
                    | Action::Hidden
                    | Action::Terminal
            );
            self.act(ui, b, a, on_item);
            return true;
        }
        match (ctrl_letter(k), k.sym) {
            (Some(b'l'), _) => self.loc.start(ui, b, None),
            (Some(b'a'), _) => b.select_all(),
            (Some(b'z'), _) => self.undo_last(b),
            (Some(b'i'), _) => self.info_open = !self.info_open,
            (Some(b'f'), _) if k.mods.shift => self.loc.start_search(ui, b),
            (Some(b'f'), _) => self.loc.start_filter(ui, b),
            (Some(b'q'), _) => self.quit = true,
            (_, KEY_PLUS | KEY_EQUAL | KEY_KP_ADD) if k.mods.ctrl => {
                self.grid.zoom_by(1);
            }
            (_, KEY_MINUS | KEY_KP_SUBTRACT) if k.mods.ctrl => {
                self.grid.zoom_by(-1);
            }
            (_, KEY_0) if k.mods.ctrl => self.grid.zoom = 0,
            (_, KEY_MENU) | (_, KEY_F10) if k.sym == KEY_MENU || k.mods.shift => {
                let at = self
                    .item_rects(b.sel)
                    .map(|(row, _)| (row.x + 48.0, row.y + row.h));
                if let Some(at) = at {
                    self.open_menu(ui, at, Some(b.sel), b);
                }
            }
            _ => return false,
        }
        true
    }

    /// Moving through the folder and history. True if `k` was one.
    fn motion(&mut self, b: &mut Browser, k: &Key) -> bool {
        let grid = self.mode != LIST;
        let row = if grid {
            self.grid.cols.max(1) as isize
        } else {
            1
        };
        let page = self.page as isize;
        let alt = k.mods.alt;
        if !alt && k.sym == KEY_BACKSPACE && !self.find.is_empty() {
            self.find.pop();
            self.refind(b);
            return true;
        }
        match (alt, k.sym) {
            (true, KEY_LEFT) => b.travel(true),
            (true, KEY_RIGHT) => b.travel(false),
            (true, KEY_UP) => b.up(),
            (true, KEY_HOME) => b.go(home(), None),
            (_, KEY_BACKSPACE) => b.up(),
            (_, KEY_ESCAPE) if !b.filter.is_empty() => b.set_filter(""),
            (_, KEY_ESCAPE) if b.search.is_some() => b.end_search(),
            // A file dialog: Esc cancels it, as in every other picker.
            (_, KEY_ESCAPE) if self.pick.is_some() && self.find.is_empty() => self.quit = true,
            (_, KEY_ESCAPE) => b.marked.clear(), // and the find text, below
            (_, KEY_RETURN) => self.open_entry(b),
            (_, KEY_HOME) => b.step(-isize::MAX),
            (_, KEY_END) => b.step(isize::MAX),
            (_, KEY_PAGE_UP) => b.step(-page),
            (_, KEY_PAGE_DOWN) => b.step(page),
            (_, KEY_UP) if k.mods.shift => b.extend(-row),
            (_, KEY_DOWN) if k.mods.shift => b.extend(row),
            (_, KEY_UP) => b.step(-row),
            (_, KEY_DOWN) => b.step(row),
            (_, KEY_LEFT) if grid => b.step(-1),
            (_, KEY_RIGHT) if grid => b.step(1),
            (_, KEY_LEFT) => b.up(),
            (_, KEY_RIGHT) => self.open_entry(b),
            _ => return false,
        }
        // Any move ends type-to-find.
        self.find.clear();
        true
    }

    /// Plain typing: `/` or `~` opens the path bar, anything else finds by name.
    fn typed(&mut self, ui: &mut Ui, b: &mut Browser, k: &Key) -> bool {
        let printable = !k.text.is_empty() && !k.text.chars().any(char::is_control);
        if k.mods.ctrl || k.mods.alt || !printable {
            return false;
        }
        if self.find.is_empty() && (k.text == "/" || k.text == "~") {
            self.loc.start(ui, b, Some(&k.text));
            return true;
        }
        self.find.push_str(&k.text);
        self.refind(b);
        true
    }

    /// Select the first entry starting with what was typed, else one containing it.
    fn refind(&mut self, b: &mut Browser) {
        let f = self.find.to_lowercase();
        if f.is_empty() {
            return;
        }
        let names: Vec<String> = b.entries.iter().map(|e| e.name.to_lowercase()).collect();
        let hit = names
            .iter()
            .position(|n| n.starts_with(&f))
            .or_else(|| names.iter().position(|n| n.contains(&f)));
        self.find_miss = hit.is_none();
        if let Some(i) = hit {
            b.sel = i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame::xkb::Mods;

    #[test]
    fn shortcuts_read_ctrl_letters_in_any_case() {
        let key = |sym, ctrl, shift| Key {
            sym,
            text: String::new(),
            mods: Mods {
                ctrl,
                shift,
                ..Mods::default()
            },
            repeat: false,
        };
        assert_eq!(shortcut(&key(0x63, true, false)), Some(Action::Copy));
        assert_eq!(
            shortcut(&key(0x43, true, true)),
            Some(Action::CopyPath),
            "Shift gives 'C'"
        );
        assert_eq!(shortcut(&key(0x4e, true, true)), Some(Action::NewFolder));
        assert_eq!(
            shortcut(&key(0x63, false, false)),
            None,
            "plain c is typing"
        );
        assert_eq!(
            shortcut(&key(KEY_DELETE, false, false)),
            Some(Action::Trash)
        );
    }
}
