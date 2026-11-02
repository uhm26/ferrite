//! A split pane's compact title bar: the file name (with a sand dot while it has
//! unsaved edits) and a short note on the left; on the right a Preview/Edit switch
//! for Markdown and a × that closes the pane. A 1 px line separates it from the
//! main pane. Hit testing uses the same slots as drawing.

use super::draw::{Canvas, Rect};
use super::tabbar::PAD_Y;
use super::{Renderer, Shown};
use crate::color::{self, px};
use crate::term::Term;

/// What the header shows.
pub struct Header<'a> {
    pub term: &'a Term,
    pub title: &'a str,
    pub focused: bool,
    pub dirty: bool,
    pub note: &'a str,
    /// The Preview/Edit switch, if the pane has one: Some(editing).
    pub toggle: Option<bool>,
}

/// A spot in the header.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Title,
    Preview,
    Edit,
    Close,
}

/// Cells per slot: the × and the switch's two choices (label plus a cell each side).
const CLOSE_CELLS: usize = 3;
const SWITCH: [(Hit, &str); 2] = [(Hit::Preview, "Preview"), (Hit::Edit, "Edit")];

impl Renderer {
    /// The clickable slots of a header ending at `x1`: (what, x, width).
    fn slots(&self, x1: usize, toggle: bool) -> Vec<(Hit, usize, usize)> {
        let cw = self.font.cell_w;
        let mut x = x1.saturating_sub(CLOSE_CELLS * cw);
        let mut out = vec![(Hit::Close, x, CLOSE_CELLS * cw)];
        if toggle {
            for (hit, label) in SWITCH.iter().rev() {
                let w = (label.len() + 2) * cw;
                x = x.saturating_sub(w);
                out.push((*hit, x, w));
            }
        }
        out
    }

    /// What is at `x` in a header ending at `x1`.
    pub fn header_hit(&self, x: usize, x1: usize, toggle: bool) -> Hit {
        let slots = self.slots(x1, toggle);
        slots
            .iter()
            .find(|s| (s.1..s.1 + s.2).contains(&x))
            .map_or(Hit::Title, |s| s.0)
    }

    /// Draw the header at y = `self.top` over `x0..x1`, if it changed since this
    /// buffer showed it (`shown.bar` remembers). Returns damage.
    pub fn pane_header(&mut self, canvas: &mut Canvas, h: &Header, (x0, x1): (usize, usize), shown: &mut Shown) -> Option<Rect> {
        let state = format!("{}{:?}{x0}", h.dirty, h.toggle);
        let key = (vec![h.title.to_string(), h.note.to_string(), state], usize::from(h.focused));
        if shown.bar.as_ref() == Some(&key) {
            return None;
        }
        shown.bar = Some(key);
        let (y, line) = (self.top, px(self.theme.tab_line));
        let bg = color::glass(self.theme.tab_bg, h.term.palette.alpha);
        let strip = Rect { x: x0, y, w: x1.saturating_sub(x0), h: self.bar_height() };
        canvas.fill(strip, bg);
        let below = canvas.h.saturating_sub(y);
        canvas.fill(Rect { x: x0.saturating_sub(1), y, w: 1, h: below }, line);
        let slots = self.slots(x1, h.toggle.is_some());
        let room = slots.last().map_or(x1, |s| s.1).saturating_sub(x0);
        self.header_name(canvas, h, (Rect { w: room, ..strip }, bg));
        for &(hit, x, w) in &slots {
            self.header_slot(canvas, h, (hit, Rect { x, w, ..strip }), bg);
        }
        Some(Rect { x: x0.saturating_sub(1), y, w: strip.w + 1, h: below })
    }

    /// The name, a sand dot after it while unsaved (after, so the name never
    /// shifts), and the note, in `r`.
    fn header_name(&mut self, canvas: &mut Canvas, h: &Header, (r, bg): (Rect, u32)) {
        let t = self.theme;
        let fg = px(if h.focused { t.tab_active } else { t.tab_inactive });
        let n = self.label(canvas, r, h.title, (fg, bg));
        if n == 0 {
            return;
        }
        let cw = self.font.cell_w;
        let mut at = r.x + (n + 1) * cw;
        let rest = |at: usize| Rect { x: at, w: (r.x + r.w).saturating_sub(at), ..r };
        if h.dirty {
            self.label(canvas, rest(at), "●", (px(t.cursor), bg));
            at += 2 * cw;
        }
        if !h.note.is_empty() {
            self.label(canvas, rest(at), h.note, (px(t.tab_inactive), bg));
        }
    }

    /// The ×, or one side of the switch (the chosen one on a raised capsule).
    fn header_slot(&mut self, canvas: &mut Canvas, h: &Header, (hit, r): (Hit, Rect), bg: u32) {
        let t = self.theme;
        let (text, on) = match hit {
            Hit::Close => ("×", false),
            Hit::Preview => ("Preview", h.toggle == Some(false)),
            Hit::Edit => ("Edit", h.toggle == Some(true)),
            Hit::Title => return,
        };
        let back = if on { px(t.tab_line) } else { bg };
        if on {
            canvas.pill(Rect { y: r.y + PAD_Y, h: r.h - 2 * PAD_Y, ..r }, (back, bg));
        }
        self.label(canvas, r, text, (px(if on { t.tab_active } else { t.tab_inactive }), back));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::font::Font;

    #[test]
    fn hits_match_the_slots() {
        let cfg = Config::default();
        let r = Renderer::new(Font::new(&cfg.font, 11.0).unwrap(), 0, cfg.theme);
        let (cw, x1) = (r.font.cell_w, 100 * r.font.cell_w);
        assert_eq!(r.header_hit(x1 - 1, x1, true), Hit::Close);
        assert_eq!(r.header_hit(x1 - 4 * cw, x1, true), Hit::Edit);
        assert_eq!(r.header_hit(x1 - 10 * cw, x1, true), Hit::Preview);
        assert_eq!(r.header_hit(x1 - 10 * cw, x1, false), Hit::Title);
        assert_eq!(r.header_hit(0, x1, true), Hit::Title);
    }
}
