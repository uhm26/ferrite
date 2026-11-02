//! The app shell, the frame every app window shares: a full-height [`Sidebar`] on its
//! own surface, a toolbar band above the content, a status band below it, and the
//! content in a rounded pane set into that surface (owner: square inner corners looked
//! "hard"). The app draws the sidebar first, then [`Shell::new`] places the rest.

use super::paint::mix; // mix(0, c, a): c at opacity a (premultiplied, glass-safe)
use super::theme::*;
use super::{Align, Icon, Rect, Ui};

/// The toolbar band: one control with 8 above and below.
pub const BAR_H: f32 = CONTROL_H + 2.0 * S8;
/// The status band.
pub const STATUS_H: f32 = ROW_H;
/// Sidebar width when open, until the owner drags its edge.
pub const SIDEBAR_W: f32 = 208.0;
/// How narrow and wide dragging may make it.
const MIN_W: f32 = 160.0;
const MAX_W: f32 = 400.0;
/// Folded to icons: one control-wide entry with 8 each side.
const FOLDED_W: f32 = CONTROL_H + 2.0 * S8;
/// Grab zone on each side of the edge, px.
const EDGE: f32 = 4.0;
/// A heading line above a section.
const HEADING_H: f32 = 28.0;

/// Where the parts of the window go, for a sidebar of a given width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shell {
    /// Room for one row of controls, 12 in from the sides.
    pub toolbar: Rect,
    /// The rounded content pane (already drawn).
    pub pane: Rect,
    /// The status band, full width right of the sidebar.
    pub status: Rect,
    /// Right of the pane, as tall: an optional side panel (0 wide when none).
    pub aside: Rect,
}

impl Shell {
    /// Lay the window out beside a sidebar `side_w` wide (drawn already) with an aside
    /// `aside_w` wide, and draw the pane.
    pub fn new(ui: &mut Ui, side_w: f32, aside_w: f32) -> Shell {
        let (w, h) = ui.p.size();
        let cw = (w - side_w).max(0.0);
        let pane = Rect::new(
            side_w,
            BAR_H,
            (cw - S8 - aside_w).max(0.0),
            (h - BAR_H - STATUS_H).max(0.0),
        );
        let window = ui.pal.window;
        ui.p.rect(pane, R_PANEL, window);
        Shell {
            toolbar: Rect::new(side_w + S12, S8, cw - 2.0 * S12, CONTROL_H),
            pane,
            status: Rect::new(side_w, h - STATUS_H, cw, STATUS_H),
            aside: Rect::new(pane.x + pane.w, BAR_H, aside_w, pane.h),
        }
    }

    /// The status band's left and right halves, 12 in from its sides (text goes left
    /// aligned in the first, right aligned in the second).
    pub fn status_halves(&self) -> (Rect, Rect) {
        let r = self.status;
        let inner = Rect::new(r.x + S12, r.y, r.w - 2.0 * S12, r.h);
        let half = Rect {
            w: (inner.w / 2.0).round(),
            ..inner
        };
        (
            half,
            Rect {
                x: half.x + half.w,
                ..half
            },
        )
    }
}

/// One sidebar entry. Entries with the same `section` in a row share its heading.
#[derive(Clone, Copy)]
pub struct SideItem<'a> {
    pub icon: Icon,
    pub label: &'a str,
    pub section: &'a str,
}

/// Draws a footer at the bottom of the sidebar: `(ui, rect, opacity)`. It fades as
/// the sidebar folds and is skipped when it would not clear the last entry.
pub type Footer<'f> = (f32, &'f mut dyn FnMut(&mut Ui, Rect, f32));

/// The full-height column of entries. When the content beside it would get narrower
/// than `content_min` it folds to icons; the change glides. Its right edge drags.
pub struct Sidebar {
    /// Open width; the owner drags the edge to change it.
    pub width: f32,
    /// Room the content beside the column needs: the column folds to icons before it
    /// would squeeze the content below that, so a wider window never shows less
    /// (owner 2026-11-01: columns vanished as the column opened).
    pub content_min: f32,
    /// The edge is being dragged.
    dragging: bool,
    /// The pointer is on the edge (or dragging): show the resize cursor.
    on_edge: bool,
    /// Width drawn last frame (where drops land).
    shown_w: f32,
}

impl Default for Sidebar {
    fn default() -> Self {
        Sidebar {
            width: SIDEBAR_W,
            content_min: 0.0,
            dragging: false,
            on_edge: false,
            shown_w: SIDEBAR_W,
        }
    }
}

/// Entry rects for a column `w` wide, and a heading (y, text) above each section.
fn layout<'a>(items: &[SideItem<'a>], w: f32) -> (Vec<Rect>, Vec<(f32, &'a str)>) {
    let mut heads = Vec::new();
    let mut y = S16;
    let mut section = None;
    let rects = items
        .iter()
        .map(|it| {
            if section != Some(it.section) {
                if section.is_some() {
                    y += S12; // gap between groups ≥ 1.5× the gap inside one
                }
                section = Some(it.section);
                heads.push((y, it.section));
                y += HEADING_H;
            }
            let r = Rect::new(S8, y, w - 2.0 * S8, CONTROL_H);
            y += CONTROL_H + 2.0;
            r
        })
        .collect();
    (rects, heads)
}

impl Sidebar {
    /// The width the column is heading to in a window `win_w` wide (it glides there).
    pub fn target_w(&self, win_w: f32) -> f32 {
        if self.opens(win_w) {
            self.width
        } else {
            FOLDED_W
        }
    }

    /// Open (names shown) in a window `win_w` wide.
    fn opens(&self, win_w: f32) -> bool {
        win_w >= self.width + self.content_min
    }

    /// The entry under `pos` as drawn last frame (a drop target): index and row.
    pub fn at(&self, items: &[SideItem], pos: (f32, f32)) -> Option<(usize, Rect)> {
        let (rects, _) = layout(items, self.shown_w);
        rects.into_iter().enumerate().find(|(_, r)| r.contains(pos))
    }

    /// Entry `i`'s row as drawn last frame (to put a button on it, e.g. eject).
    pub fn rect(&self, items: &[SideItem], i: usize) -> Option<Rect> {
        layout(items, self.shown_w).0.get(i).copied()
    }

    /// The pointer is on the resize edge or dragging it.
    pub fn resizing(&self) -> bool {
        self.on_edge || self.dragging
    }

    /// Drag the right edge to set the width (no spring: it follows the hand).
    fn drag(&mut self, ui: &mut Ui) {
        let (win_w, h) = ui.p.size();
        let edge = Rect::new(self.width - EDGE, 0.0, 2.0 * EDGE, h);
        let input = &ui.s.input;
        self.on_edge = edge.contains(input.pos);
        if self.on_edge && input.pressed {
            self.dragging = true;
            ui.s.input.pressed = false; // the press is ours, not an entry's
        }
        if !ui.s.input.down {
            self.dragging = false;
        }
        if self.dragging {
            // Never so wide that the content gets too narrow and the column folds.
            let max = MAX_W.min(win_w - self.content_min).max(MIN_W);
            self.width = ui.s.input.pos.0.clamp(MIN_W, max).round();
        }
    }

    /// Paint the window in the sidebar surface (the toolbar and status bands share
    /// it), then the column: headings, `items` with `current` on a gliding surface,
    /// and the footer. Returns the column's width this frame (it glides when folding)
    /// and the entry clicked.
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        items: &[SideItem],
        current: Option<usize>,
        footer: Option<Footer>,
    ) -> (f32, Option<usize>) {
        let (win_w, h) = ui.p.size();
        let open = ui.spring("sidebar/open", f32::from(self.opens(win_w)), GLIDE);
        if open > 0.5 {
            self.drag(ui);
        } else {
            (self.dragging, self.on_edge) = (false, false);
        }
        let w = (FOLDED_W + (self.width - FOLDED_W) * open).round();
        self.shown_w = w;
        let side = ui.pal.sidebar;
        ui.p.clear(side);
        let col = Rect::new(0.0, 0.0, w, h);
        ui.p.rect(col, 0.0, side);
        ui.p.clip(Some(col));
        // Text keeps its open-width layout as the column folds: it fades and the edge
        // cuts it (never squashed, never gone at once).
        let full = self.width - 2.0 * S16;
        let (rects, heads) = layout(items, w);
        if open > 0.0 {
            for (y, title) in heads {
                let ink = ui.heading();
                let ink = ink.color(mix(0, ink.color, open));
                ui.text(Rect::new(S16, y, full, HEADING_H), title, ink, Align::Left);
            }
        }
        // The current entry's surface glides between entries (and fades off-list).
        let raise = ui.pal.raise;
        ui.slide("sidebar/current", current.map(|i| rects[i]), raise);
        let bottom = rects.last().map_or(0.0, |r| r.y + r.h);
        let mut go = None;
        for (i, (it, r)) in items.iter().zip(rects).enumerate() {
            let label = (it.icon, it.label, open);
            if ui.nav(&format!("sidebar/{i}"), r, label, current == Some(i)) {
                go = Some(i);
            }
        }
        // The footer only where it clears the last entry (short windows).
        if let Some((fh, draw)) = footer {
            let y = h - S16 - fh;
            if open > 0.0 && y >= bottom + S16 {
                draw(ui, Rect::new(S16, y, full, fh), open);
            }
        }
        ui.p.clip(None);
        (w, go)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{State, Text, theme::FAMILY};
    use crate::window::Canvas;

    /// Press on the edge, move, release: the width follows the pointer, clamped.
    #[test]
    fn dragging_the_edge_resizes() {
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut side = Sidebar::default();
        let mut px = vec![0u32; 900 * 300];
        let mut frame = |s: &mut State, p: &mut Sidebar, pos: (f32, f32), down: Option<bool>| {
            s.input.pos = pos;
            if let Some(d) = down {
                s.input.button(d);
            }
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 900,
                    h: 300,
                },
                0.05,
            );
            p.drag(&mut ui);
            ui.end();
        };
        frame(&mut s, &mut side, (SIDEBAR_W + 1.0, 100.0), Some(true));
        assert!(side.resizing());
        frame(&mut s, &mut side, (300.0, 100.0), None);
        assert_eq!(side.width, 300.0);
        frame(&mut s, &mut side, (900.0, 100.0), None);
        assert_eq!(side.width, MAX_W, "clamped");
        frame(&mut s, &mut side, (900.0, 100.0), Some(false));
        frame(&mut s, &mut side, (100.0, 100.0), None);
        assert_eq!(side.width, MAX_W, "released: no longer follows");
    }

    /// A new section starts a heading and a wider gap; `at` finds entries by row.
    #[test]
    fn sections_get_headings_and_hits() {
        let it = |label, section| SideItem {
            icon: Icon::Home,
            label,
            section,
        };
        let items = [it("a", "One"), it("b", "One"), it("c", "Two")];
        let (rects, heads) = layout(&items, SIDEBAR_W);
        assert_eq!(
            heads.iter().map(|h| h.1).collect::<Vec<_>>(),
            ["One", "Two"]
        );
        assert_eq!(rects[1].y - rects[0].y, CONTROL_H + 2.0);
        assert_eq!(rects[2].y - rects[1].y, CONTROL_H + 2.0 + S12 + HEADING_H);
        let side = Sidebar::default();
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert_eq!(side.at(&items, mid(rects[2])).map(|h| h.0), Some(2));
        assert_eq!(side.at(&items, (1.0, 1.0)), None);
    }
}
