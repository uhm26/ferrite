//! The folder as a list on one panel: column titles (Name, Size, Type, Modified),
//! then one row per entry with its icon. Narrow panels keep Name and Size only.
//! Only rows in view are drawn, so big folders cost what fits on screen.

use frame::ui::paint::mix;
use frame::ui::theme::*;
use frame::ui::{Align, Rect, Ui};

use super::scroll::Scroll;
use crate::browse::Browser;
use crate::{dir, sys};

/// Right-hand columns: title, width. Name takes the rest.
const COLS: [(&str, f32); 3] = [("Size", 80.0), ("Type", 120.0), ("Modified", 104.0)];
/// Narrower panels show only Name and Size.
const ALL_COLS_W: f32 = 600.0;

/// Draw the list in `panel`; returns the entry clicked, if any.
pub fn draw(ui: &mut Ui, b: &Browser, panel: Rect, scroll: &mut Scroll) -> Option<usize> {
    ui.panel(panel);
    let inner = panel.inset(S4);
    let cols = if inner.w >= ALL_COLS_W {
        &COLS[..]
    } else {
        &COLS[..1]
    };
    header(ui, Rect { h: ROW_H, ..inner }, cols);
    let rows = Rect::new(inner.x, inner.y + ROW_H, inner.w, inner.h - ROW_H);
    if b.entries.is_empty() {
        super::empty(ui, rows);
        return None;
    }
    let content = b.entries.len() as f32 * ROW_H;
    let key = format!("scroll/list/{}", b.cwd.display());
    let sel = (b.sel as f32 * ROW_H, ROW_H);
    let off = scroll.offset(ui, &key, sel, (rows.h, content));
    let first = (off / ROW_H) as usize;
    let last = (((off + rows.h) / ROW_H).ceil() as usize).min(b.entries.len());
    let now = sys::local(unix_now());
    let mut clicked = None;
    for i in first..last {
        ui.p.clip(Some(rows));
        let r = Rect::new(rows.x, rows.y + i as f32 * ROW_H - off, rows.w, ROW_H);
        if row(ui, b, (i, r), (cols, now)) {
            clicked = Some(i);
        }
    }
    ui.p.clip(None);
    let over = panel.contains(ui.s.input.pos);
    ui.scroll_thumb("scroll/list/thumb", rows, (off, rows.h, content), over);
    clicked
}

/// Column titles in the heading style.
fn header(ui: &mut Ui, r: Rect, cols: &[(&str, f32)]) {
    let ink = ui.heading();
    let name_x = r.x + S8 + ICON + S8;
    let cols_w: f32 = cols.iter().map(|c| c.1).sum();
    ui.text(Rect::new(name_x, r.y, r.w, r.h), "Name", ink, Align::Left);
    let mut x = r.x + r.w - cols_w - S8;
    for (i, (title, w)) in cols.iter().enumerate() {
        // Size reads right-aligned (digits line up); the rest left.
        let align = if i == 0 { Align::Right } else { Align::Left };
        let cell = Rect::new(x, r.y, w - S16, r.h);
        ui.text(cell, title, ink, align);
        x += w;
    }
}

/// One entry: frame's row for hover and selection, then icon, name and columns.
/// True if clicked.
fn row(
    ui: &mut Ui,
    b: &Browser,
    (i, r): (usize, Rect),
    (cols, now): (&[(&str, f32)], sys::Local),
) -> bool {
    let id = format!("row/{i}");
    let clicked = ui.row(&id, r, i == b.sel);
    // A selected row lifts its secondary text (SUB is too faint on SELECTED).
    let sel = ui.spring(&id, f32::from(i == b.sel), SNAP);
    let sub = mix(ui.pal.sub, ui.pal.text, sel);
    let e = &b.entries[i];
    let icon_r = Rect::new(r.x + S8, r.y, ICON, r.h);
    ui.icon(super::icon(e), icon_r, ICON, sub);
    let cols_w: f32 = cols.iter().map(|c| c.1).sum();
    let name_x = icon_r.x + ICON + S8;
    let name_w = r.x + r.w - cols_w - S8 - S16 - name_x;
    let body = ui.body();
    ui.text(
        Rect::new(name_x, r.y, name_w, r.h),
        &e.name,
        body,
        Align::Left,
    );
    let small = ui.small().color(sub);
    let size = if e.dir {
        String::new()
    } else {
        dir::human(e.size)
    };
    let texts = [size, dir::kind(e).1, dir::date_of(e.modified, now)];
    let mut x = r.x + r.w - cols_w - S8;
    for (c, ((_, w), t)) in cols.iter().zip(&texts).enumerate() {
        let align = if c == 0 { Align::Right } else { Align::Left };
        ui.text(Rect::new(x, r.y, w - S16, r.h), t, small, align);
        x += w;
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
    use frame::ui::text::{Ink, MEDIUM, Text};

    /// The widest texts each column shows must fit it (they are cut with … otherwise).
    #[test]
    fn widest_values_fit_their_columns() {
        let mut t = Text::new(FAMILY).unwrap();
        let ink = Ink::new(SMALL, MEDIUM, 0);
        let widest = [
            ("1023 MB", 0),
            ("QuickTime video", 1),
            ("Shell script", 1),
            ("30 May 2025", 2),
            ("28 Sep 2026", 2),
        ];
        for (text, col) in widest {
            let room = COLS[col].1 - S16;
            let w = t.width(text, ink);
            assert!(w <= room, "{text:?} is {w:.1} px, column {col} has {room}");
        }
    }
}
