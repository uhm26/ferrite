//! Pointer gestures on the entries besides clicks. A press on empty space starts a
//! selection box. A press on an entry that then moves a few px drags the selection:
//! out of the window as a `text/uri-list` (Discord, Firefox, another file manager
//! take it), or onto a folder row, a place in the sidebar, or the pane (= this
//! folder) of our own window. A drag from another app lands the same way.
//! Ours moves by default, Ctrl copies; other apps' files are copied.

use std::path::{Path, PathBuf};

use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, Rect, State, Ui};
use frame::window::Canvas;

use crate::browse::Browser;
use crate::ops::{Op, trash};
use crate::uri;
use super::grid::Grid;
use super::list::List;
use crate::browse::tabs::Tabs;
use crate::view::{LIST, View};

/// How far a pressed entry must move before it is dragged, px.
const DRAG_PX: f32 = 6.0;

#[derive(Default)]
pub struct Drags {
    /// A selection box: where it started, and whether it adds (Ctrl).
    band: Option<((f32, f32), bool)>,
    /// An entry pressed, and where: moving far enough drags the selection.
    press: Option<(usize, (f32, f32))>,
    /// Files to start dragging (the app hands them to the compositor).
    start: Option<Vec<PathBuf>>,
    /// Our files being dragged right now.
    ours: Vec<PathBuf>,
    /// Files from somewhere are dragged over the window.
    pub(in crate::view) over: bool,
}

impl View {
    /// The selection box and drag starts, and the drop target while files are
    /// dragged over us. `panel` holds the entries.
    pub(in crate::view) fn gestures(&mut self, ui: &mut Ui, b: &mut Browser, panel: Rect) {
        self.drag_start(ui, b, panel);
        self.band(ui, b, panel);
        if let Some(r) = self.drop_hint {
            let text = ui.pal.text;
            if r.w <= 2.0 || r.h <= 2.0 {
                ui.p.rect(r, 1.0, alpha(text, 0.85)); // between entries: a line
            } else {
                let radius = if r == self.pane { R_PANEL } else { RADIUS };
                ui.p.rect(r, radius, alpha(text, 0.06));
                ui.p.stroke(r, radius, 1.0, alpha(text, 0.6));
            }
        }
    }

    /// A press on an entry, then a move past [`DRAG_PX`]: drag the selection (the
    /// pressed entry alone if it wasn't selected).
    fn drag_start(&mut self, ui: &Ui, b: &mut Browser, panel: Rect) {
        let i = &ui.s.input;
        if i.pressed && panel.contains(i.pos) {
            self.drags.press = self.item_at(i.pos, b.entries.len()).map(|k| (k, i.pos));
        }
        if !i.down {
            self.drags.press = None;
        }
        let Some((k, (x, y))) = self.drags.press else {
            return;
        };
        if (i.pos.0 - x).hypot(i.pos.1 - y) < DRAG_PX {
            return;
        }
        self.drags.press = None;
        if !b.is_selected(k) {
            b.click(k, (false, false));
        }
        self.drags.start = Some(b.selected_paths());
    }

    /// The selection box: a press on empty space in the list (not on an entry or the
    /// column titles) starts it; dragging selects every entry it touches.
    fn band(&mut self, ui: &mut Ui, b: &mut Browser, panel: Rect) {
        let i = &ui.s.input;
        let (pos, n) = (i.pos, b.entries.len());
        let header = self.mode == LIST && pos.1 < panel.y + ROW_H;
        if i.pressed && panel.contains(pos) && self.item_at(pos, n).is_none() && !header {
            self.drags.band = Some((pos, i.mods.ctrl));
        }
        let Some((start, add)) = self.drags.band else {
            return;
        };
        if !i.down {
            self.drags.band = None;
            return;
        }
        let r = Rect::new(
            start.0.min(pos.0),
            start.1.min(pos.1),
            (pos.0 - start.0).abs(),
            (pos.1 - start.1).abs(),
        );
        let hits: Vec<usize> = (0..n)
            .filter(|&k| self.item_rects(k).is_some_and(|(row, _)| overlaps(row, r)))
            .collect();
        b.mark(&hits, add);
        if let Some(&last) = hits.last() {
            b.sel = last;
        }
        ui.p.clip(Some(panel));
        ui.p.rect(r.round(), 3.0, alpha(ACCENT, 0.12));
        ui.p.stroke(r.round(), 3.0, 1.0, alpha(ACCENT, 0.5));
        ui.p.clip(None);
    }

    /// A drag to start: its `text/uri-list` and the pill that follows the pointer.
    /// From now on the files are ours until the drag ends.
    pub fn take_drag(&mut self, st: &mut State) -> Option<Dragged> {
        let paths = self.drags.start.take()?;
        let list = uri::list(&paths).into_bytes();
        let (icon, label) = match paths.as_slice() {
            [p] if p.is_dir() => (Icon::FolderFill, crate::ops::name_of(p)),
            [p] => (Icon::File, crate::ops::name_of(p)),
            many => (Icon::File, format!("{} items", many.len())),
        };
        self.drags.ours = paths;
        let (px, w, h) = pill(st, icon, &label);
        Some(Dragged { list, px, w, h })
    }

    /// Our drag ended (dropped anywhere, or cancelled).
    pub fn drag_ended(&mut self) {
        self.drags.ours.clear();
    }

    /// We drag files of our own (a drop on us needs no reading).
    pub fn dragging_ours(&self) -> bool {
        !self.drags.ours.is_empty()
    }

    /// Files are dragged over the window (true), or left it / were dropped.
    pub fn drag_over(&mut self, files: bool) {
        self.drags.over = files;
    }

    /// Where a drop at `pos` would go, and what to mark: a place in the sidebar, a
    /// tab, a folder entry (over its middle: outlined), else the folder of the pane
    /// under it (a line between the entries there). None: nowhere, or nothing to do
    /// (our own files moved to the folder they are in).
    pub fn drop_target(&self, tabs: &Tabs, pos: (f32, f32)) -> Option<(Rect, PathBuf)> {
        if let Some(t) = self.places.at(pos) {
            return Some(t);
        }
        if let Some((r, p)) = self.tab_rects.iter().find(|t| t.0.contains(pos)) {
            return (!self.ours_in(p)).then(|| (*r, p.clone()));
        }
        if let (Some((r, _)), Some(o)) = (&self.other_drop, tabs.other())
            && r.contains(pos)
        {
            return self.pane_drop(tabs.get(o), self.side_geo(), *r, pos);
        }
        self.pane_drop(tabs.cur(), (&self.list, &self.grid), self.pane, pos)
    }

    /// Our own dragged files are all in `dir` already.
    fn ours_in(&self, dir: &Path) -> bool {
        self.drags.ours.iter().any(|p| p.parent() == Some(dir))
    }

    /// A drop at `pos` in a pane `r` showing `b` (laid out by `list` / `grid`): into
    /// the folder entry it is over the middle of, else into `b`'s folder, marked by a
    /// line at the nearest gap between entries (the pane's outline where there is none).
    fn pane_drop(&self, b: &Browser, (list, grid): (&List, &Grid), r: Rect, pos: (f32, f32)) -> Option<(Rect, PathBuf)> {
        if !r.contains(pos) {
            return None;
        }
        let n = b.entries.len();
        let at = |i: usize| if self.mode == LIST { list.rects(i) } else { grid.rects(i) };
        let hit = if self.mode == LIST { list.at(pos, n) } else { grid.at(pos, n) };
        let hit = hit.and_then(|i| Some((i, at(i)?.0)));
        let gap = match hit {
            Some((i, row)) => {
                let e = &b.entries[i];
                let p = b.cwd.join(&e.os);
                if e.dir && middle(row, pos, self.mode == LIST) && !self.drags.ours.contains(&p) {
                    return Some((row, p));
                }
                Some(gap_line(row, pos, self.mode == LIST))
            }
            // Below the last entry: the gap after it.
            None => n.checked_sub(1).and_then(at).map(|(row, _)| gap_line(row, (pos.0, f32::MAX), self.mode == LIST)),
        };
        (!self.ours_in(&b.cwd)).then(|| (gap.unwrap_or(r), b.cwd.clone()))
    }

    /// Our own drag dropped on `dir`.
    pub fn drop_ours(&mut self, b: &mut Browser, dir: PathBuf, copy: bool) {
        let paths = std::mem::take(&mut self.drags.ours);
        self.drop_files(b, paths, dir, copy);
    }

    /// Files dropped on `dir`: into the Trash if that's where, else copied or moved
    /// (asking first about names already taken there). A folder of this list that
    /// takes them gets selected (else the moved-away selection falls to the top row).
    pub fn drop_files(&mut self, b: &mut Browser, paths: Vec<PathBuf>, dir: PathBuf, copy: bool) {
        if paths.is_empty() {
            return;
        }
        if let Some(i) = b.entries.iter().position(|e| b.cwd.join(&e.os) == dir) {
            b.click(i, (false, false));
        }
        if dir == trash::files_dir() {
            self.run(Op::Trash(paths));
        } else {
            self.transfer(paths, dir, copy);
        }
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// What a drag carries: the uri list, and the pill under the pointer (`w * h` px).
pub struct Dragged {
    pub list: Vec<u8>,
    pub px: Vec<u32>,
    pub w: usize,
    pub h: usize,
}

/// Longest pill: longer names are cut.
const PILL_MAX_W: f32 = 280.0;

/// The drag pill: a capsule with the icon and the name (or "3 items").
fn pill(st: &mut State, icon: Icon, label: &str) -> (Vec<u32>, usize, usize) {
    let tw = st.text.width(label, Ink::new(BODY, REGULAR, 0)).ceil();
    let w = (S12 + ICON + S8 + tw + S16).min(PILL_MAX_W);
    let (wu, hu) = (w as usize, CONTROL_H as usize);
    let mut px = vec![0u32; wu * hu];
    let mut ui = st.begin(
        Canvas {
            px: &mut px,
            w: wu,
            h: hu,
        },
        0.0,
    );
    let r = Rect::new(0.0, 0.0, w, CONTROL_H);
    let (raise, line, text) = (ui.pal.raise, ui.pal.line, ui.pal.text);
    ui.p.rect(r, CONTROL_H / 2.0, raise);
    ui.p.stroke(r, CONTROL_H / 2.0, 1.0, line);
    ui.icon(icon, Rect::new(S12, 0.0, ICON, CONTROL_H), ICON, text);
    let tx = S12 + ICON + S8;
    let body = ui.body();
    ui.text(
        Rect::new(tx, 0.0, w - tx - S12, CONTROL_H),
        label,
        body,
        Align::Left,
    );
    ui.end();
    (px, wu, hu)
}

/// Over the middle of an entry (its middle half; a tile's middle 60 % across).
fn middle(r: Rect, (x, y): (f32, f32), list: bool) -> bool {
    if list {
        y > r.y + r.h * 0.25 && y < r.y + r.h * 0.75
    } else {
        x > r.x + r.w * 0.2 && x < r.x + r.w * 0.8
    }
}

/// A 2 px line in the gap nearest `pos`: above or below a row, left or right of a tile.
fn gap_line(r: Rect, (x, y): (f32, f32), list: bool) -> Rect {
    if list {
        let gy = if y < r.y + r.h / 2.0 { r.y } else { r.y + r.h };
        Rect::new(r.x + S8, gy - 1.0, r.w - 2.0 * S8, 2.0)
    } else {
        let gx = if x < r.x + r.w / 2.0 { r.x } else { r.x + r.w };
        Rect::new(gx - 1.0, r.y + S8, 2.0, r.h - 2.0 * S8)
    }
}
