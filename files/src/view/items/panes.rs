//! Split view: two tabs side by side (a middle click on a tab puts it beside this
//! one). The pane in use has the view's list, grid and scroll; the other pane keeps
//! its own in [`Side`] and they trade places when focus moves, so each pane keeps
//! its scroll and springs. Any press in the other pane focuses it first, so a click,
//! a drag or a right-click there act there. Its selection doesn't show while it waits.

use frame::ui::theme::*;
use frame::ui::{Rect, Ui};

use super::grid::Grid;
use super::list::List;
use super::scroll::Scroll;
use crate::browse::Browser;
use crate::browse::tabs::Tabs;
use std::path::PathBuf;

use crate::view::{LIST, View};

/// The pane not in use: its list, grid and scroll, and which tabs own what.
pub struct Side {
    list: List,
    grid: Grid,
    scroll: Scroll,
    /// The folder this pane showed last (switching panes must not re-aim scrolling).
    shown: PathBuf,
    /// The tabs whose state is in the view (`main`) and here (`tab`).
    main: u64,
    tab: Option<u64>,
    /// The other pane sat on the left (kept while the split closes).
    left: bool,
}

impl Default for Side {
    fn default() -> Self {
        Side {
            list: List::tagged("b/"),
            grid: Grid::tagged("b/"),
            scroll: Scroll::default(),
            shown: PathBuf::new(),
            main: 0,
            tab: None,
            left: false,
        }
    }
}

/// Where the panes go this frame: the one in use, and the other one's full rect and
/// the part of it showing (it slides in from its edge as the split opens).
pub type Panes = (Rect, Option<(Rect, Rect)>);

impl View {
    /// Trade list, grid, scroll and last folder with the second pane (focus moved).
    fn swap_side(&mut self) {
        let s = &mut self.side;
        std::mem::swap(&mut self.list, &mut s.list);
        std::mem::swap(&mut self.grid, &mut s.grid);
        std::mem::swap(&mut self.scroll, &mut s.scroll);
        std::mem::swap(&mut self.shown, &mut s.shown);
        std::mem::swap(&mut s.main, s.tab.get_or_insert(0));
    }

    /// Keep each pane's state with its tab: if the current tab's state is in the
    /// second pane (focus moved), swap it in.
    pub(in crate::view) fn follow_panes(&mut self, (cur, other): (u64, Option<u64>)) {
        if self.side.tab == Some(cur) && other.is_some() {
            self.swap_side();
        }
        if other != self.side.tab {
            // A different tab beside us: fresh state for it.
            let fresh = Side::default();
            let s = &mut self.side;
            (s.list, s.grid, s.scroll, s.shown) = (fresh.list, fresh.grid, fresh.scroll, fresh.shown);
        }
        (self.side.main, self.side.tab) = (cur, other);
    }

    /// Where the entries go in `panel` (window `w` wide): all of it, or with the other
    /// pane on the `left` / right, one half each. Opening and closing glide: this pane
    /// narrows while the other slides in from its edge at its final width. Sets whether
    /// the list shows all columns, from the width it is heading to.
    pub(in crate::view) fn panes(&mut self, ui: &mut Ui, (w, panel): (f32, Rect), left: Option<bool>) -> Panes {
        let info_to = if self.info_open { crate::view::info::INFO_W } else { 0.0 };
        let mut pane_to = w - self.places.side.target_w(w) - S8 - info_to - 2.0 * S4;
        if let Some(l) = left {
            self.side.left = l;
            pane_to = (pane_to - S8) / 2.0;
        }
        self.list.wide = pane_to >= super::list::ALL_COLS_W;
        let t = ui.spring("split/open", f32::from(left.is_some()), GLIDE);
        let half = ((panel.w - S8) / 2.0).floor();
        let mine_w = (panel.w + (half - panel.w) * t).round();
        let other_left = self.side.left;
        let mine = Rect {
            x: if other_left { panel.x + panel.w - mine_w } else { panel.x },
            w: mine_w,
            ..panel
        };
        self.pane = if t > 0.0 { mine } else { self.pane };
        let Some(_) = left else {
            return (mine, None);
        };
        let shown_w = (panel.w - mine_w - S8).max(0.0);
        let (full_x, vis_x) = if other_left {
            (panel.x + shown_w - half, panel.x)
        } else {
            (panel.x + panel.w - shown_w, panel.x + panel.w - shown_w)
        };
        let full = Rect { x: full_x, w: half, ..panel };
        let vis = Rect { x: vis_x, w: shown_w, ..panel };
        (mine, Some((full, vis)))
    }

    /// The other pane, laid out in `full` but showing only `vis`, with `ob`'s folder,
    /// drawn without its selection (it isn't the pane in use). Drops land in it.
    pub(in crate::view) fn other_pane(&mut self, ui: &mut Ui, ob: &mut Browser, (full, vis): (Rect, Rect)) {
        let (wide, zoom) = (self.list.wide, self.grid.zoom);
        self.swap_side();
        (self.list.wide, self.grid.zoom) = (wide, zoom);
        (self.list.passive, self.grid.passive) = (true, true);
        ui.p.bound(Some(vis));
        // A press here focused this pane before drawing, so nothing is clicked here.
        if self.mode == LIST {
            self.list.draw(ui, ob, full, &mut self.scroll);
        } else {
            self.grid.draw(ui, ob, (full, &mut self.thumbs), &mut self.scroll);
        }
        ui.p.bound(None);
        (self.list.passive, self.grid.passive) = (false, false);
        self.swap_side();
        self.other_drop = Some((vis, ob.cwd.clone()));
        let line = ui.pal.line;
        let mid = if vis.x > self.pane.x { vis.x - S4 } else { vis.x + vis.w + S4 };
        ui.p.rect(Rect::new(mid.round(), vis.y, 1.0, vis.h), 0.0, line);
    }

    /// A press (any button) in the other pane, by last frame's layout: that pane
    /// becomes the one in use before this frame is drawn, so the press acts there.
    /// Returns where a left press was, to select what is under it at once.
    pub(in crate::view) fn focus_on_press(&mut self, ui: &Ui, tabs: &mut Tabs) -> Option<(f32, f32)> {
        let (r, _) = self.other_drop.as_ref()?;
        if self.menu.is_some() || self.prompt.is_some() {
            return None;
        }
        let i = &ui.s.input;
        let left = i.pressed.then_some(i.pos);
        let any = [left, self.right_click, self.middle_click];
        if !any.into_iter().flatten().any(|p| r.contains(p)) {
            return None;
        }
        tabs.focus_other();
        self.other_drop = None;
        left
    }

    /// A left press that just focused this pane: select the entry under it now (a
    /// plain click selects on release, which showed the old selection for a moment).
    pub(in crate::view) fn select_pressed(&self, ui: &Ui, b: &mut Browser, at: (f32, f32)) {
        let m = ui.s.input.mods;
        if m.ctrl || m.shift {
            return; // those change the selection on release; twice would undo it
        }
        if let Some(i) = self.item_at(at, b.entries.len()) {
            b.click(i, (false, false));
        }
    }

    /// F5 / F6: copy or move the selection into the other pane's folder.
    pub(in crate::view) fn send_to_other(&mut self, tabs: &Tabs, copy: bool) {
        let Some(o) = tabs.other() else {
            return;
        };
        let dir = tabs.folder(o);
        let paths = tabs.cur().selected_paths();
        if !paths.is_empty() {
            self.transfer(paths, dir, copy);
        }
    }

    /// The other pane's list and grid (their layout from last frame).
    pub(in crate::view) fn side_geo(&self) -> (&List, &Grid) {
        (&self.side.list, &self.side.grid)
    }
}
