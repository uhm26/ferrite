//! The folder as a list in the pane: column titles over a hairline, then one row per
//! entry with its icon. Like Nemo, the name fills the width and Size, Type and
//! Modified sit at the right edge (owner's pick 2026-11-01). Each value column is as
//! wide as its widest text and they sit an even `GAP` apart, so the block reads as
//! one aligned unit (owner: fixed widths "looks so uncentered"). Narrow lists keep
//! Name and Size only. Only rows in view are drawn.

use frame::ui::paint::mix;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Rect, Ui};

use super::scroll::Scroll;
use crate::browse::Browser;
use crate::dir::{self, Entry, SortBy};
use crate::sys;

const TITLES: [&str; 3] = ["Size", "Type", "Modified"];
/// Space between columns, and before the name column ends.
const GAP: f32 = S24;
/// Narrower lists show only Name and Size.
pub const ALL_COLS_W: f32 = 600.0;
/// Entries measured for column widths (a huge folder costs one bounded pass).
const MEASURE: usize = 4096;
/// Sort arrow beside the active column title.
const ARROW: f32 = 12.0;

/// Where things go in a row.
struct Cols {
    name_x: f32,
    name_w: f32,
    /// Left edge and width of each value column (Size first).
    at: Vec<(f32, f32)>,
    /// How much Type and Modified show (0-1): they slide out past the edge, fading.
    fade: f32,
}

/// Column widths, measured once per folder load.
#[derive(Default)]
pub struct List {
    /// `Browser::generation` they were measured for, and the widths.
    measured: Option<(u64, [f32; 3])>,
    /// A column title clicked this frame (the view sorts by it).
    pub sort_click: Option<SortBy>,
    /// All value columns, not just Size: set by the view from where the layout is
    /// heading, so a gliding sidebar never flips them mid-way.
    pub wide: bool,
    /// Prefix for its widget names: the split view's second list must not share
    /// hover and selection springs with the first ("" or "b/").
    pub tag: &'static str,
    /// Drawn as the pane not in use: no selection shown.
    pub passive: bool,
    /// Last frame's rows area, scroll offset and name column (x, w): for the
    /// right-click target, the menu key and the rename field.
    geo: Option<(Rect, f32, f32, f32)>,
}

impl List {
    /// One whose widget names start with `tag` (the split view's second pane).
    pub fn tagged(tag: &'static str) -> Self {
        List {
            tag,
            ..List::default()
        }
    }

    /// Draw the list in `area`; returns the entry clicked, if any.
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        b: &Browser,
        area: Rect,
        scroll: &mut Scroll,
    ) -> Option<usize> {
        let widths = self.widths(ui, b);
        let wide = ui.spring(
            &format!("{}list/wide", self.tag),
            f32::from(self.wide),
            GLIDE,
        ); // a breakpoint: glides (ui §4)
        let c = layout(area, widths, wide);
        ui.p.clip(Some(area)); // columns leaving slide under the edge
        self.sort_click = header(ui, (self.tag, Rect { h: ROW_H, ..area }), &c, b.sort);
        ui.p.clip(None);
        let rows = Rect::new(area.x, area.y + ROW_H + S4, area.w, area.h - ROW_H - S4);
        self.geo = None;
        if b.entries.is_empty() {
            super::empty(ui, rows);
            return None;
        }
        let content = b.entries.len() as f32 * ROW_H;
        let key = format!("{}scroll/list/{}", self.tag, b.cwd.display());
        let sel = (b.sel as f32 * ROW_H, ROW_H);
        let off = scroll.offset(ui, &key, sel, (rows.h, content));
        self.geo = Some((rows, off, c.name_x, c.name_w));
        let first = (off / ROW_H) as usize;
        let last = (((off + rows.h) / ROW_H).ceil() as usize).min(b.entries.len());
        let now = sys::local(unix_now());
        let mut clicked = None;
        for i in first..last {
            ui.p.clip(Some(rows));
            let r = Rect::new(rows.x, rows.y + i as f32 * ROW_H - off, rows.w, ROW_H);
            if row(ui, b, ((self.tag, self.passive), i, r), (&c, now)) {
                clicked = Some(i);
            }
        }
        ui.p.clip(None);
        let over = area.contains(ui.s.input.pos);
        let thumb = format!("{}scroll/list/thumb", self.tag);
        ui.scroll_thumb(&thumb, rows, (off, rows.h, content), over);
        clicked
    }

    /// The entry under `pos`, from last frame's layout.
    pub fn at(&self, pos: (f32, f32), n: usize) -> Option<usize> {
        let (rows, off, ..) = self.geo?;
        let i = ((pos.1 - rows.y + off) / ROW_H).floor();
        (rows.contains(pos) && i >= 0.0 && (i as usize) < n).then_some(i as usize)
    }

    /// Entry `i`'s row and its name cell, if on screen last frame.
    pub fn rects(&self, i: usize) -> Option<(Rect, Rect)> {
        let (rows, off, name_x, name_w) = self.geo?;
        let row = Rect::new(rows.x, rows.y + i as f32 * ROW_H - off, rows.w, ROW_H);
        let visible = row.y >= rows.y - 1.0 && row.y + row.h <= rows.y + rows.h + 1.0;
        let name = Rect::new(name_x - S4, row.y + 2.0, name_w + S4, row.h - 4.0);
        visible.then_some((row, name))
    }

    /// Widest text per value column (titles included), for the folder shown.
    fn widths(&mut self, ui: &mut Ui, b: &Browser) -> [f32; 3] {
        if let Some((g, w)) = self.measured
            && g == b.generation
        {
            return w;
        }
        let (small, heading) = (ui.small(), ui.heading());
        let now = sys::local(unix_now());
        let mut w = TITLES.map(|t| ui.s.text.width(t, heading));
        for e in b.entries.iter().take(MEASURE) {
            for (k, t) in values(e, now).iter().enumerate() {
                w[k] = w[k].max(ui.s.text.width(t, small));
            }
        }
        let w = w.map(f32::ceil);
        self.measured = Some((b.generation, w));
        w
    }
}

/// Value columns pinned at the right edge, `GAP` apart; the name takes the rest.
/// As `wide` goes 1 → 0 (a narrower window), Type and Modified slide right past the
/// edge until Size sits there, so nothing jumps.
fn layout(r: Rect, widths: [f32; 3], wide: f32) -> Cols {
    let mut at = place(r.x + r.w - S12, &widths);
    let tail = at[2].0 + at[2].1 - (at[0].0 + at[0].1);
    let shift = ((1.0 - wide) * tail).round();
    for col in &mut at {
        col.0 += shift;
    }
    let name_x = r.x + S8 + ICON + S8;
    let name_w = (at[0].0 - GAP - name_x).max(0.0).round();
    Cols {
        name_x,
        name_w,
        at,
        fade: wide,
    }
}

/// Lay columns of `widths` right to left, ending at `right`, `GAP` apart.
fn place(right: f32, widths: &[f32]) -> Vec<(f32, f32)> {
    let mut x = right;
    let mut at: Vec<(f32, f32)> = widths
        .iter()
        .rev()
        .map(|&w| {
            x -= w;
            let col = (x.round(), w);
            x -= GAP;
            col
        })
        .collect();
    at.reverse();
    at
}

/// The Size, Type and Modified texts of an entry.
fn values(e: &Entry, now: sys::Local) -> [String; 3] {
    let size = match (e.dir, e.items) {
        (true, Some(n)) => dir::items(n),
        (true, None) => String::new(),
        (false, _) => dir::human(e.size),
    };
    [size, dir::kind(e).1, dir::date_of(e.modified, now)]
}

/// Size reads right-aligned (digits line up); the rest left.
fn align(col: usize) -> Align {
    if col == 0 { Align::Right } else { Align::Left }
}

/// Column titles in the heading style over a hairline (space alone can't separate
/// a header from the first row at this density). Each title sorts by its column; the
/// active one carries an arrow. Returns the column clicked.
fn header(ui: &mut Ui, (tag, r): (&str, Rect), c: &Cols, sort: (SortBy, bool)) -> Option<SortBy> {
    const BY: [SortBy; 4] = [SortBy::Name, SortBy::Size, SortBy::Kind, SortBy::Modified];
    let cells = std::iter::once((c.name_x, c.name_w, "Name", Align::Left)).chain(
        c.at.iter()
            .enumerate()
            .map(|(k, &(x, w))| (x, w, TITLES[k], align(k))),
    );
    let mut hit = None;
    for (k, (x, w, title, al)) in cells.enumerate() {
        let cell = Rect::new(x, r.y, w, r.h);
        if ui.clicked(&format!("{tag}sort/{k}"), cell) {
            hit = Some(BY[k]);
        }
        let on = sort.0 == BY[k];
        let base = if on { ui.pal.text } else { ui.pal.sub };
        let ink = ui
            .heading()
            .color(alpha(base, if k >= 2 { c.fade } else { 1.0 }));
        ui.text(cell, title, ink, al);
        if on {
            let tw = ui.s.text.width(title, ink).ceil();
            let ax = if al == Align::Right {
                x + w - tw - S4 - ARROW
            } else {
                x + tw + S4
            };
            let icon = if sort.1 { Icon::Down } else { Icon::Up };
            ui.icon(icon, Rect::new(ax, r.y, ARROW, r.h), ARROW, ink.color);
        }
    }
    let line = ui.pal.line;
    ui.p.rect(Rect::new(r.x, r.y + r.h, r.w, 1.0), 0.0, line);
    hit
}

/// One entry: frame's row for hover and selection, then icon, name and columns.
/// True if clicked.
fn row(
    ui: &mut Ui,
    b: &Browser,
    ((tag, passive), i, r): ((&str, bool), usize, Rect),
    (c, now): (&Cols, sys::Local),
) -> bool {
    let id = format!("{tag}row/{i}");
    let on = !passive && b.is_selected(i);
    let clicked = ui.row(&id, r, on);
    // A selected row lifts its secondary text (SUB is too faint on SELECTED).
    let sel = ui.spring(&id, f32::from(on), SNAP);
    let sub = mix(ui.pal.sub, ui.pal.text, sel);
    let e = &b.entries[i];
    let icon_r = Rect::new(r.x + S8, r.y, ICON, r.h);
    ui.icon(
        crate::view::icon(e),
        icon_r,
        ICON,
        crate::view::icon_color(ui, e, sel),
    );
    let body = ui.body();
    ui.text(
        Rect::new(c.name_x, r.y, c.name_w, r.h),
        &e.name,
        body,
        Align::Left,
    );
    let small = ui.small().color(sub);
    for (k, (&(x, w), t)) in c.at.iter().zip(&values(e, now)).enumerate() {
        let ink = if k >= 1 {
            small.color(alpha(sub, c.fade))
        } else {
            small
        };
        ui.text(Rect::new(x, r.y, w, r.h), t, ink, align(k));
    }
    clicked
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowing_slides_type_and_modified_out_past_the_edge() {
        let r = Rect::new(0.0, 0.0, 1012.0, 400.0); // columns end at x 1000
        let end = |c: &Cols, k: usize| c.at[k].0 + c.at[k].1;
        let (all, size_only) = (
            layout(r, [60.0, 90.0, 50.0], 1.0),
            layout(r, [60.0, 90.0, 50.0], 0.0),
        );
        assert_eq!((end(&all, 2), end(&size_only, 0)), (1000.0, 1000.0));
        assert!(size_only.at[1].0 > 1000.0, "Type is past the edge");
        assert!(size_only.name_w > all.name_w, "the name takes the room");
    }

    #[test]
    fn columns_end_at_the_edge_with_even_gaps() {
        let at = place(1000.0, &[60.0, 90.0, 50.0]);
        assert_eq!(at, [(752.0, 60.0), (836.0, 90.0), (950.0, 50.0)]);
        for pair in at.windows(2) {
            assert_eq!(pair[1].0 - (pair[0].0 + pair[0].1), GAP);
        }
    }
}
