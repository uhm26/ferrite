//! The one-line text field: a caret and a selection, set by keys (←/→ Home/End, with
//! Shift to select, Ctrl+A) and by the mouse (click places the caret, drag selects),
//! and scrolling that keeps the caret in view. Copy and paste belong to the app (the
//! clipboard is the window's): it reads [`Ui::field_selection`], and calls
//! [`Ui::paste_into`] or [`Ui::cut`].

use std::ops::Range;

use super::ctx::{Ui, id};
use super::paint::{Rect, mix};
use super::text::Ink;
use super::theme::*;

/// xkbcommon keysyms (xkbcommon-keysyms.h).
const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_HOME: u32 = 0xff50;
const KEY_LEFT: u32 = 0xff51;
const KEY_RIGHT: u32 = 0xff53;
const KEY_END: u32 = 0xff57;
const KEY_DELETE: u32 = 0xffff;
const KEY_A: u32 = 0x61;
const CLEAR: u32 = 0;

/// Caret and selection of the focused field, as byte offsets into its text (on
/// char boundaries once clamped), and how far its text is scrolled, px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edit {
    pub caret: usize,
    pub anchor: usize,
    pub off: f32,
}

impl Edit {
    /// Everything selected (caret at the end).
    pub const ALL: Edit = Edit {
        caret: usize::MAX,
        anchor: 0,
        off: 0.0,
    };
    /// Caret at the end, nothing selected.
    pub const END: Edit = Edit {
        caret: usize::MAX,
        anchor: usize::MAX,
        off: 0.0,
    };

    /// Keep caret and anchor inside `s`, on char boundaries.
    fn clamp(&mut self, s: &str) {
        self.caret = floor(s, self.caret);
        self.anchor = floor(s, self.anchor);
    }

    fn range(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// Put the caret at `i`; without `extend` the selection collapses there.
    fn to(&mut self, i: usize, extend: bool) {
        self.caret = i;
        if !extend {
            self.anchor = i;
        }
    }

    /// Replace the selection in `s` with `text`; the caret lands after it.
    fn insert(&mut self, s: &mut String, text: &str) {
        let r = self.range();
        s.replace_range(r.clone(), text);
        self.to(r.start + text.len(), false);
    }
}

fn floor(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn prev(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(j, _)| j)
}

fn next(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

impl Ui<'_> {
    /// A one-line text field. A click focuses it and places the caret, a drag
    /// selects; typing edits `value`; Return ([`Ui::submitted`]) or Esc lets go.
    /// After [`Ui::take_focus_all`] the whole text is selected. Returns whether
    /// `value` changed.
    pub fn field(&mut self, name: &str, r: Rect, value: &mut String, placeholder: &str) -> bool {
        let r = r.round();
        let ink = self.body();
        self.mouse(name, r, value, ink);
        let focused = self.focused(name);
        let changed = focused && self.edit(value);
        let f = self.spring(name, f32::from(self.focused(name)), SNAP);
        let pal = self.pal;
        self.p.rect(r, RADIUS, mix(pal.raise, pal.hover, f));
        // Focus shows as a ring: keyboard users must see where typing goes.
        self.p.stroke(r, RADIUS, 1.0, mix(CLEAR, pal.sub, f));
        let y = self.s.text.center(r.y, r.h, ink);
        let x0 = r.x + S12;
        self.p.clip(Some(r.inset(1.0)));
        if value.is_empty() {
            self.label((x0, y), placeholder, ink.color(pal.sub));
        }
        if !self.focused(name) {
            self.label((x0, y), value, ink);
            self.p.clip(None);
            return changed;
        }
        self.scroll(value, ink, r.w - 2.0 * S12);
        // Caret, selection edges and scroll glide (SNAP spring) instead of jumping
        // letter to letter; an emptied selection shrinks into the caret.
        let e = self.s.edit;
        let sel = e.range();
        let at = |ui: &mut Self, i: usize| ui.s.text.width(&value[..i], ink);
        let targets = [
            e.off,
            at(self, sel.start),
            at(self, sel.end),
            at(self, e.caret),
        ];
        let [off, a, b, c] = [0, 1, 2, 3].map(|k| {
            let part = ["off", "a", "b", "c"][k];
            self.spring(&format!("{name}/{part}"), targets[k], SNAP)
        });
        let x = x0 - off;
        let cap = self.s.text.metrics(ink.px, ink.style).cap;
        if b - a > 0.5 {
            let band = Rect::new(x + a - 1.0, y - cap - 4.0, b - a + 2.0, cap + 8.0);
            self.p.rect(band, 3.0, alpha(ACCENT, 0.35));
        }
        self.label((x, y), value, ink);
        // The caret: accent, no blink (idle stays at 0 CPU).
        let cx = (x + c).round();
        self.p
            .rect(Rect::new(cx, y - cap - 2.0, 2.0, cap + 4.0), 0.0, ACCENT);
        self.p.clip(None);
        changed
    }

    /// Press: focus and place the caret. Held: drag to select.
    fn mouse(&mut self, name: &str, r: Rect, value: &str, ink: Ink) {
        let over = self.over(r);
        self.s.text_cursor |= over;
        let press = over && self.s.input.pressed;
        if !press && !(self.focused(name) && self.held(name)) {
            return;
        }
        if press {
            if !self.focused(name) {
                self.take_focus(name);
            }
            self.s.focus_taken = true;
            self.s.active = Some(id(name));
        }
        let x = self.s.input.pos.0 - (r.x + S12) + self.s.edit.off;
        let i = self.index_at(value, ink, x);
        self.s.edit.to(i, !press);
    }

    /// The char boundary nearest to `x` px into `value`.
    fn index_at(&mut self, value: &str, ink: Ink, x: f32) -> usize {
        let mut acc = 0.0;
        for (i, c) in value.char_indices() {
            let w = self.s.text.width(&value[i..i + c.len_utf8()], ink);
            if x < acc + w / 2.0 {
                return i;
            }
            acc += w;
        }
        value.len()
    }

    /// Scroll so the caret stays inside `avail` px, without empty space at the end.
    fn scroll(&mut self, value: &str, ink: Ink, avail: f32) {
        self.s.edit.clamp(value);
        let e = self.s.edit;
        let cx = self.s.text.width(&value[..e.caret], ink);
        let total = self.s.text.width(value, ink);
        let mut off = e.off.min(cx).max(cx - avail);
        off = off.min((total - avail).max(0.0)).max(0.0);
        self.s.edit.off = off;
    }

    /// Apply this frame's keys to `value`. True if it changed. Ctrl combinations
    /// other than Ctrl+A are left for the app (copy, cut, paste …).
    fn edit(&mut self, value: &mut String) -> bool {
        let mut changed = false;
        let mut rest = Vec::new();
        for k in std::mem::take(&mut self.s.input.keys) {
            self.s.edit.clamp(value);
            let (e, shift) = (&mut self.s.edit, k.mods.shift);
            let sel = e.range();
            match k.sym {
                KEY_RETURN => self.s.submitted = self.s.focus.take(),
                KEY_ESCAPE => self.s.focus = None,
                KEY_LEFT if !shift && !sel.is_empty() => e.to(sel.start, false),
                KEY_RIGHT if !shift && !sel.is_empty() => e.to(sel.end, false),
                KEY_LEFT => e.to(prev(value, e.caret), shift),
                KEY_RIGHT => e.to(next(value, e.caret), shift),
                KEY_HOME => e.to(0, shift),
                KEY_END => e.to(value.len(), shift),
                KEY_BACKSPACE | KEY_DELETE if !sel.is_empty() => {
                    e.insert(value, "");
                    changed = true;
                }
                KEY_BACKSPACE if e.caret > 0 => {
                    e.anchor = prev(value, e.caret);
                    e.insert(value, "");
                    changed = true;
                }
                KEY_DELETE if e.caret < value.len() => {
                    e.anchor = next(value, e.caret);
                    e.insert(value, "");
                    changed = true;
                }
                KEY_A if k.mods.ctrl => {
                    *e = Edit {
                        off: e.off,
                        ..Edit::ALL
                    }
                }
                _ if k.mods.ctrl => rest.push(k),
                _ if !k.text.is_empty() && !k.text.chars().any(char::is_control) => {
                    e.insert(value, &k.text);
                    changed = true;
                }
                _ => {}
            }
        }
        self.s.input.keys = rest;
        changed
    }

    /// The focused field's selected text in `value` (empty if none).
    pub fn field_selection<'v>(&mut self, value: &'v str) -> &'v str {
        self.s.edit.clamp(value);
        &value[self.s.edit.range()]
    }

    /// Remove the focused field's selection from `value` and return it (Ctrl+X).
    pub fn cut(&mut self, value: &mut String) -> String {
        let taken = self.field_selection(value).to_string();
        self.s.edit.insert(value, "");
        taken
    }

    /// Insert pasted `text` at the caret of a focused field's `value`, replacing the
    /// selection. Line breaks become spaces (one-line field).
    pub fn paste_into(&mut self, value: &mut String, text: &str) {
        let one_line: String = text
            .trim_end_matches(['\n', '\r'])
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        self.s.edit.clamp(value);
        self.s.edit.insert(value, &one_line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Click places the caret near the pointer; dragging selects.
    #[test]
    fn mouse_places_caret_and_drag_selects() {
        use crate::ui::{State, Text};
        use crate::window::Canvas;
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut px = vec![0u32; 300 * 40];
        let mut value = String::from("hello world");
        let r = Rect::new(0.0, 0.0, 300.0, 32.0);
        let mut frame = |s: &mut State, x: f32, down: Option<bool>, value: &mut String| {
            s.input.pos = (x, 16.0);
            if let Some(d) = down {
                s.input.button(d);
            }
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 300,
                    h: 40,
                },
                0.0,
            );
            ui.field("f", r, value, "");
            let sel = ui.field_selection(value).to_string();
            ui.end();
            sel
        };
        frame(&mut s, S12 + 1.0, Some(true), &mut value); // press at the start
        assert_eq!(s.edit.caret, 0);
        let sel = frame(&mut s, 299.0, None, &mut value); // drag to the end
        assert_eq!(sel, "hello world");
        frame(&mut s, 299.0, Some(false), &mut value);
        assert!(s.focus.is_some(), "still focused after the click");
    }

    /// Collapsing a selection animates (the band shrinks over frames), then settles.
    #[test]
    fn selection_glides_then_settles() {
        use crate::ui::{State, Text};
        use crate::window::Canvas;
        use crate::xkb::{Key, Mods};
        let right = Key {
            sym: KEY_RIGHT,
            text: String::new(),
            mods: Mods::default(),
            repeat: false,
        };
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut px = vec![0u32; 300 * 40];
        let mut value = String::from("/home/user");
        let r = Rect::new(0.0, 0.0, 300.0, 32.0);
        let mut frame = |s: &mut State, keys: Vec<Key>, open: bool, value: &mut String| -> bool {
            s.input.keys = keys;
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 300,
                    h: 40,
                },
                0.016,
            );
            if open {
                ui.take_focus_all("f");
            }
            ui.field("f", r, value, "");
            ui.end();
            s.animating()
        };
        frame(&mut s, vec![], true, &mut value);
        assert!(
            !frame(&mut s, vec![], false, &mut value),
            "opening jumps, no slide"
        );
        assert!(
            frame(&mut s, vec![right], false, &mut value),
            "collapse animates"
        );
        let settled = (0..60).any(|_| !frame(&mut s, vec![], false, &mut value));
        assert!(settled);
    }

    #[test]
    fn edit_moves_on_char_boundaries_and_replaces_selections() {
        let mut s = String::from("aé/b");
        let mut e = Edit::END;
        e.clamp(&s);
        assert_eq!(e.caret, s.len());
        e.to(prev(&s, e.caret), false); // before 'b'
        e.to(prev(&s, e.caret), true); // select '/'
        e.to(prev(&s, e.caret), true); // and 'é' (2 bytes)
        assert_eq!(&s[e.range()], "é/");
        e.insert(&mut s, "X");
        assert_eq!((s.as_str(), e.caret, e.anchor), ("aXb", 2, 2));
        assert_eq!(floor("aé", 2), 1, "never inside a char");
    }
}
