//! The tab bar: a strip above the grid, shown with two or more tabs. Flat, rice
//! style: surface background, 1 px hairlines, dim inactive titles. The active tab
//! opens into the terminal below (same background, no bottom line); a short sand pill
//! under its title is the one accent.

use super::draw::{Canvas, Rect};
use super::{Renderer, Shown, View};
use crate::color::{self, px};
use crate::term::Term;

/// Space above and below the title text, px.
const PAD_Y: usize = 4; // 4-grid (ui skill)
/// Widest a tab gets, in cells (text plus one cell of space on each side).
const MAX_TAB_CELLS: usize = 24;
/// Height of the accent pill under the active tab's title, px.
const ACCENT: usize = 2; // owner: 4 px was "a thick ass line"
/// Radius of the active tab's corners, px.
const CORNER: usize = 7;

impl Renderer {
    pub fn bar_height(&self) -> usize {
        self.font.cell_h + 2 * PAD_Y
    }

    /// Each tab's (x, width): as wide as its text, at most [`MAX_TAB_CELLS`]; if
    /// they don't fit in `w`, no tab is wider than an equal share.
    pub fn tab_spans(&self, labels: &[String], w: usize) -> Vec<(usize, usize)> {
        let cw = self.font.cell_w;
        let want = |i: usize, l: &String| (text(i, l).chars().count() + 2).min(MAX_TAB_CELLS) * cw;
        let mut widths: Vec<usize> = labels.iter().enumerate().map(|(i, l)| want(i, l)).collect();
        if widths.iter().sum::<usize>() > w {
            let share = (w / labels.len().max(1)).max(1);
            widths.iter_mut().for_each(|x| *x = (*x).min(share));
        }
        let mut x = 0;
        widths
            .into_iter()
            .map(|tw| {
                x += tw;
                (x - tw, tw)
            })
            .collect()
    }

    /// The tab under `x` in a bar `w` px wide.
    pub fn tab_at(&self, x: f64, w: usize, labels: &[String]) -> Option<usize> {
        let x = x.max(0.0) as usize;
        self.tab_spans(labels, w).iter().position(|&(tx, tw)| x < tx + tw)
    }

    /// Draw the bar if it differs from what this buffer shows. Returns damage.
    pub(super) fn tab_bar(
        &mut self,
        term: &Term,
        view: &View,
        canvas: &mut Canvas,
        shown: &mut Shown,
    ) -> Option<Rect> {
        if view.tabs.len() < 2 || self.top == 0 {
            return None;
        }
        let same = |(t, a): &(Vec<String>, usize)| t == view.tabs && *a == view.active_tab;
        if shown.bar.as_ref().is_some_and(same) {
            return None;
        }
        shown.bar = Some((view.tabs.to_vec(), view.active_tab));
        let t = self.theme;
        let glass = |c: u32| color::glass(c, term.palette.alpha);
        let bg = term.palette.bg_px();
        let (w, h) = (canvas.w, self.top);
        let bar = Rect { x: 0, y: 0, w, h };
        canvas.fill(bar, glass(t.tab_bg));
        let spans = self.tab_spans(view.tabs, w);
        let active = view.active_tab;
        for (i, (title, &(x, tw))) in view.tabs.iter().zip(&spans).enumerate() {
            let r = Rect { x, y: 0, w: tw, h };
            if i == active {
                continue; // drawn last, its corners reach into the neighbours
            }
            if i + 1 != active {
                let sep = Rect { x: x + tw - 1, y: PAD_Y, w: 1, h: h - 2 * PAD_Y };
                canvas.fill(sep, px(t.tab_line));
            }
            self.label(canvas, r, &text(i, title), (px(t.tab_inactive), glass(t.tab_bg)));
        }
        if let (Some(title), Some(&(x, tw))) = (view.tabs.get(active), spans.get(active)) {
            let r = Rect { x, y: 0, w: tw, h };
            active_shape(canvas, r, (bg, glass(t.tab_bg)));
            let n = self.label(canvas, r, &text(active, title), (px(t.tab_active), bg));
            // The one accent: an M3 tab indicator, a pill as wide as the title, on the
            // tab's bottom edge.
            let cw = self.font.cell_w;
            let pill = Rect { x: x + cw, y: h - ACCENT, w: n * cw, h: ACCENT };
            canvas.pill(pill, (px(t.cursor), bg));
        }
        Some(bar)
    }

    /// One tab's text, one cell in from the left, cut with `…` if too long.
    /// Returns how many cells it took.
    fn label(&mut self, canvas: &mut Canvas, r: Rect, text: &str, (fg, bg): (u32, u32)) -> usize {
        let (cw, ch) = (self.font.cell_w, self.font.cell_h);
        let room = r.w.saturating_sub(2 * cw) / cw;
        if room == 0 {
            return 0; // a squeezed tab: no text rather than text spilling into the next
        }
        let mut chars: Vec<char> = text.chars().collect();
        if chars.len() > room {
            chars.truncate(room.saturating_sub(1));
            chars.push('…');
        }
        for (k, &c) in chars.iter().enumerate() {
            if c == ' ' {
                continue; // the background is already there
            }
            let cell = Rect {
                x: r.x + cw * (k + 1),
                y: PAD_Y,
                w: cw,
                h: ch,
            };
            let g = self.cache.get(&mut self.font, c, 0, 1);
            canvas.glyph(cell, &g.data, bg, fg);
        }
        chars.len()
    }
}

/// What a tab shows: its number and title.
fn text(i: usize, title: &str) -> String {
    format!("{} {title}", i + 1)
}

/// The active tab as a surface growing out of the terminal below: rounded top
/// corners, and inverted corners at the bottom that flare into the terminal's
/// edge (rice vision: edge-grown surfaces, inverted corners). Antialiased.
fn active_shape(canvas: &mut Canvas, r: Rect, (tab, bar): (u32, u32)) {
    let rad = CORNER.min(r.w / 2).min(r.h / 2);
    canvas.fill(r, tab);
    let (rf, x0, x1, h) = (rad as f32, r.x as f32, (r.x + r.w) as f32, r.h as f32);
    let sq = |x: usize, y: usize| Rect { x, y, w: rad, h: rad };
    // Top corners: tab inside the circle, bar outside.
    canvas.round(sq(r.x, 0), (x0 + rf, rf), rf, (tab, bar));
    canvas.round(sq(r.x + r.w - rad, 0), (x1 - rf, rf), rf, (tab, bar));
    // Bottom flares, just outside the tab: bar inside the circle, tab outside.
    let y = r.h - rad;
    if r.x >= rad {
        canvas.round(sq(r.x - rad, y), (x0 - rf, h - rf), rf, (bar, tab));
    }
    canvas.round(sq(r.x + r.w, y), (x1 + rf, h - rf), rf, (bar, tab));
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::font::Font;
    use crate::render::Renderer;

    #[test]
    fn tabs_fit_their_text() {
        let cfg = Config::default();
        let r = Renderer::new(Font::new(&cfg.font, 11.0).unwrap(), 0, cfg.theme);
        let cw = r.font.cell_w;
        let labels = ["a".to_string(), "x".repeat(100)];
        let spans = r.tab_spans(&labels, 1000 * cw);
        assert_eq!(spans[0], (0, 5 * cw)); // "1 a" + one cell each side
        assert_eq!(spans[1], (5 * cw, super::MAX_TAB_CELLS * cw));
        let tight = r.tab_spans(&labels, 10 * cw); // too narrow: equal shares at most
        assert!(tight.iter().all(|&(_, w)| w <= 5 * cw));
        assert_eq!(r.tab_at((6 * cw) as f64, 1000 * cw, &labels), Some(1));
        assert_eq!(r.tab_at((40 * cw) as f64, 1000 * cw, &labels), None);
    }
}
