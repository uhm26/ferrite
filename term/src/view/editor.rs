//! Edit mode: keys, mouse and drawing for a [`Doc`] being edited. The visible
//! lines are written as VT bytes (line numbers, syntax colors, the selection on its
//! own background, a beam cursor), so the pane draws like any other.

use std::fmt::Write as _;

use super::buffer::{Pos, advance};
use super::{Doc, Kind, Mode, Style, code, sgr};
use crate::xkb::Key;

/// What the app has to do after a key in a file pane.
pub enum Outcome {
    /// Handled here; the pane is redrawn.
    Done,
    /// Scroll the preview by this many rows (up is positive).
    Scroll(isize),
    Copy(String),
    Close,
}

// Keysyms (xkbcommon-keysyms.h).
const BACKSPACE: u32 = 0xff08;
const TAB: u32 = 0xff09;
const RETURN: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;
const HOME: u32 = 0xff50;
const LEFT: u32 = 0xff51;
const UP: u32 = 0xff52;
const RIGHT: u32 = 0xff53;
const DOWN: u32 = 0xff54;
const PAGE_UP: u32 = 0xff55;
const PAGE_DOWN: u32 = 0xff56;
const END: u32 = 0xff57;
const KP_ENTER: u32 = 0xff8d;
const DELETE: u32 = 0xffff;

const DIM: u8 = 8;

impl Doc {
    /// A key in this pane, `rows` tall.
    pub fn key(&mut self, k: &Key, rows: usize) -> Outcome {
        let ctrl = k.mods.ctrl && !k.mods.alt;
        if ctrl
            && char::from_u32(k.sym).is_some_and(|c| c.eq_ignore_ascii_case(&'e'))
            && self.toggle().is_some()
        {
            self.set_mode(if self.mode == Mode::Edit {
                Mode::Preview
            } else {
                Mode::Edit
            });
            return Outcome::Done;
        }
        if self.mode == Mode::Preview {
            return preview_key(k, rows);
        }
        (self.confirm, self.follow) = (false, true);
        if ctrl && let Some(out) = self.ctrl_key(k) {
            return out;
        }
        self.edit_key(k, rows, ctrl);
        Outcome::Done
    }

    fn ctrl_key(&mut self, k: &Key) -> Option<Outcome> {
        let b = &mut self.buf;
        match char::from_u32(k.sym)?.to_ascii_lowercase() {
            's' => self.save(),
            'z' if k.mods.shift => drop(b.redo()),
            'z' => drop(b.undo()),
            'y' => drop(b.redo()),
            'a' => {
                let end = b.last();
                b.go((0, 0), false);
                b.go(end, true);
            }
            'c' => return Some(b.selected_text().map_or(Outcome::Done, Outcome::Copy)),
            'x' => {
                let text = b.selected_text();
                b.erase(false, false);
                return Some(text.map_or(Outcome::Done, Outcome::Copy));
            }
            _ => return None,
        }
        self.note.clear();
        Some(Outcome::Done)
    }

    fn edit_key(&mut self, k: &Key, rows: usize, ctrl: bool) {
        let (b, shift) = (&mut self.buf, k.mods.shift);
        let page = rows.saturating_sub(1).max(1) as isize;
        match k.sym {
            LEFT if b.selection().is_some() && !shift => {
                b.go(b.selection().map_or(b.cur, |s| s.0), false)
            }
            RIGHT if b.selection().is_some() && !shift => {
                b.go(b.selection().map_or(b.cur, |s| s.1), false)
            }
            LEFT => b.go(
                if ctrl {
                    b.word_left(b.cur)
                } else {
                    b.left(b.cur)
                },
                shift,
            ),
            RIGHT => b.go(
                if ctrl {
                    b.word_right(b.cur)
                } else {
                    b.right(b.cur)
                },
                shift,
            ),
            UP => b.vertical(-1, shift),
            DOWN => b.vertical(1, shift),
            PAGE_UP => b.vertical(-page, shift),
            PAGE_DOWN => b.vertical(page, shift),
            HOME => b.go(if ctrl { (0, 0) } else { b.home() }, shift),
            END => b.go(if ctrl { b.last() } else { b.end() }, shift),
            BACKSPACE => b.erase(false, ctrl),
            DELETE => b.erase(true, ctrl),
            RETURN | KP_ENTER => b.newline(),
            TAB => b.insert(&self.indent),
            ESCAPE => b.anchor = None,
            _ if !ctrl
                && !k.mods.alt
                && !k.text.is_empty()
                && !k.text.chars().any(char::is_control) =>
            {
                b.insert(&k.text);
            }
            _ => {}
        }
    }

    /// Pasted text goes in at the cursor (edit mode only).
    pub fn paste(&mut self, text: &str) -> bool {
        if self.mode != Mode::Edit {
            return false;
        }
        self.buf
            .insert(&text.replace("\r\n", "\n").replace('\r', "\n"));
        (self.confirm, self.follow) = (false, true);
        true
    }

    /// A left press (`drag`: the pointer moved while held) at screen cell (col, row).
    pub fn click(&mut self, (col, row): (usize, usize), drag: bool, double: bool) {
        let l = (self.top + row).min(self.buf.line_count() - 1);
        let c = (col.saturating_sub(self.gutter) + self.left).min(usize::MAX / 2);
        let p: Pos = (l, self.buf.byte_at(l, c));
        if double {
            let (a, b) = self.buf.word_at(p);
            self.buf.go(a, false);
            self.buf.go(b, true);
        } else {
            self.buf.go(p, drag);
        }
        self.follow = true;
    }

    /// Wheel: move the view `n` lines (down is positive), the cursor stays.
    pub fn scroll(&mut self, n: isize) {
        self.top = self.top.saturating_add_signed(n).min(self.buf.line_count() - 1);
    }
}

/// Keys in a preview: scroll; Esc or q closes.
fn preview_key(k: &Key, rows: usize) -> Outcome {
    let page = rows.saturating_sub(2).max(1) as isize;
    match k.sym {
        UP => Outcome::Scroll(1),
        DOWN => Outcome::Scroll(-1),
        PAGE_UP => Outcome::Scroll(page),
        PAGE_DOWN => Outcome::Scroll(-page),
        HOME => Outcome::Scroll(isize::MAX / 2),
        END => Outcome::Scroll(-(isize::MAX / 2)),
        ESCAPE => Outcome::Close,
        _ if k.text == "q" => Outcome::Close,
        _ => Outcome::Done, // a preview takes no other keys
    }
}

impl Doc {
    /// VT bytes for the visible lines, `cols` x `rows`; `sel` is the selection
    /// color (0xRRGGBB). Scrolls first if the cursor moved out of view.
    pub fn render_edit(&mut self, cols: usize, rows: usize, sel: u32) -> Vec<u8> {
        let n = self.buf.line_count();
        self.gutter = n.to_string().len() + 1;
        let width = cols.saturating_sub(self.gutter).max(1);
        if std::mem::take(&mut self.follow) {
            self.reveal(rows, width);
        }
        self.top = self.top.min(n - 1);
        let mut out = String::from("\x1b[0m\x1b[H\x1b[2J");
        for r in 0..rows.min(n - self.top) {
            let l = self.top + r;
            let num = Style {
                fg: (l != self.buf.cur.0).then_some(DIM),
                ..Style::default()
            };
            let _ = write!(out, "\x1b[{};1H", r + 1);
            sgr(&mut out, num);
            let _ = write!(out, "{:>w$} ", l + 1, w = self.gutter - 1);
            self.line_out(&mut out, l, width, sel);
        }
        out.push_str("\x1b[0m");
        let (l, b) = self.buf.cur;
        let col = self.buf.col((l, b));
        if (self.top..self.top + rows).contains(&l) && (self.left..self.left + width).contains(&col)
        {
            let _ = write!(
                out,
                "\x1b[{};{}H\x1b[6 q\x1b[?25h",
                l - self.top + 1,
                self.gutter + col - self.left + 1
            );
        } else {
            out.push_str("\x1b[?25l");
        }
        out.into_bytes()
    }

    /// Line `l`, colored, cut to the `width` columns from `self.left`.
    fn line_out(&self, out: &mut String, l: usize, width: usize, sel: u32) {
        let text = self.buf.line(l);
        let spans = match self.kind {
            Kind::Code => code::highlight(text, code::comment_mark(self.ext())),
            Kind::Markdown => vec![super::Span::new(Style::default(), text)],
        };
        let range = self.buf.selection();
        let selected = |b: usize| range.is_some_and(|(s, e)| (l, b) >= s && (l, b) < e);
        let (mut col, mut byte, mut last) = (0, 0, None);
        let end = self.left + width;
        for span in &spans {
            for c in span.text.chars() {
                let next = advance(col, c);
                if col >= self.left && next <= end {
                    let state = (span.style, selected(byte));
                    if last != Some(state) {
                        style(out, state, sel);
                        last = Some(state);
                    }
                    if c == '\t' {
                        out.extend(std::iter::repeat_n(' ', next - col));
                    } else {
                        out.push(c);
                    }
                }
                (col, byte) = (next, byte + c.len_utf8());
            }
        }
        // A selected line end shows as one selected cell.
        if selected(byte) && l + 1 < self.buf.line_count() && (self.left..end).contains(&col) {
            style(out, (Style::default(), true), sel);
            out.push(' ');
        }
    }

    /// Scroll so the cursor is in view.
    fn reveal(&mut self, rows: usize, width: usize) {
        let (l, b) = self.buf.cur;
        let rows = rows.max(1);
        if l < self.top {
            self.top = l;
        } else if l >= self.top + rows {
            self.top = l + 1 - rows;
        }
        let col = self.buf.col((l, b));
        if col < self.left {
            self.left = col.saturating_sub(width / 4);
        } else if col >= self.left + width {
            self.left = col + 1 - width + width / 4;
        }
    }
}

/// A style plus, if selected, the selection background.
fn style(out: &mut String, (s, selected): (Style, bool), sel: u32) {
    sgr(out, s);
    if selected {
        let _ = write!(
            out,
            "\x1b[48;2;{};{};{}m",
            sel >> 16,
            (sel >> 8) & 0xff,
            sel & 0xff
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::screen;
    use super::*;
    use crate::xkb::Mods;

    fn key(sym: u32, text: &str, ctrl: bool) -> Key {
        Key {
            sym,
            text: text.into(),
            mods: Mods {
                ctrl,
                ..Mods::default()
            },
            repeat: false,
        }
    }

    fn doc(text: impl AsRef<[u8]>, name: &str) -> Doc {
        let dir = std::env::temp_dir().join(format!("term-edit-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        Doc::open(&path, 1).unwrap()
    }

    #[test]
    fn typing_saving_and_line_endings() {
        let mut d = doc("a\r\n\tb\r\n", "x.rs");
        assert_eq!(d.mode, Mode::Edit);
        for c in "hi".chars() {
            d.key(&key(c as u32, &c.to_string(), false), 10);
        }
        d.key(&key(RETURN, "\r", false), 10);
        assert!(d.buf.dirty());
        d.key(&key('s' as u32, "\x13", true), 10);
        assert!(!d.buf.dirty(), "{}", d.note);
        assert_eq!(
            std::fs::read_to_string(&d.path).unwrap(),
            "hi\r\na\r\n\tb\r\n"
        );
        std::fs::remove_dir_all(d.path.parent().unwrap()).unwrap();
    }

    #[test]
    fn draws_numbers_and_cuts_long_lines() {
        let mut d = doc("fn main() {}\nsecond line that is long\n", "y.rs");
        d.buf.go((1, 23), false);
        d.follow = true;
        let rows = screen(&d.render_edit(12, 3, 0x333333), 12);
        // 3 lines -> gutter "n " (2 cols), 10 cols of text, scrolled right to the cursor
        assert_eq!(rows[0], "1");
        assert!(
            rows[1].starts_with("2 ") && rows[1].ends_with("long"),
            "{rows:?}"
        );
        d.click((2, 0), false, false);
        assert_eq!(d.buf.cur, (0, 12));
        std::fs::remove_dir_all(d.path.parent().unwrap()).unwrap();
    }

    #[test]
    fn markdown_toggles_and_binary_is_read_only() {
        let mut d = doc("# hi\n", "z.md");
        assert_eq!((d.mode, d.toggle()), (Mode::Preview, Some(false)));
        d.key(&key('e' as u32, "\x05", true), 10);
        assert_eq!(d.mode, Mode::Edit);
        let b = doc([0u8, 0xff, b'x'], "bin");
        assert!(matches!(b.mode, Mode::Preview) && b.toggle().is_none() && !b.note.is_empty());
        std::fs::remove_dir_all(d.path.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(b.path.parent().unwrap()).unwrap();
    }
}
