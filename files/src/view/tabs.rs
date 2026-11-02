//! The tab strip along the top of the pane, shown while more than one tab is open
//! (it grows in and out). Click shows a tab, drag moves it along the strip, × closes
//! it, a middle click shows it beside this one (split view, items/panes.rs; again on
//! that tab: one pane). Files dropped on a tab go into its folder. Keys: Ctrl+T new
//! tab, Ctrl+W close (the window on the last), Ctrl+PgUp / PgDn previous / next.
//! A middle click on a folder or place opens it in a tab behind this one.

use std::path::PathBuf;

use frame::ui::paint::mix;
use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, Rect, Ui};
use frame::xkb::Key;

use super::View;
use super::keys::{self, KEY_PAGE_DOWN, KEY_PAGE_UP};
use crate::browse::tabs::Tabs;
use crate::ops::trash;

const KEY_TAB: u32 = 0xff09;
const KEY_F3: u32 = 0xffc0;
const KEY_F5: u32 = 0xffc2;
const KEY_F6: u32 = 0xffc3;
/// Tab height, and the widest a tab gets.
const TAB_H: f32 = ROW_H;
const TAB_MAX_W: f32 = 240.0;
/// The × in a tab.
const CLOSE: f32 = 20.0;
/// How far a pressed tab moves before it is dragged, px.
const DRAG_PX: f32 = 4.0;

/// A tab change asked for while drawing (the tabs aren't at hand then).
pub enum TabReq {
    /// Open this folder in a new tab; true shows it.
    Open(PathBuf, bool),
    Close(usize),
    Show(usize),
    /// Show this tab beside the current one (or stop, if it is there already).
    Split(usize),
    /// Move a tab from one place to another.
    Move(usize, usize),
}

/// One tab as the strip draws it: id, label, folder.
pub type TabLine = (u64, String, PathBuf);

/// What the strip needs to know: its lines, the current tab, and the one beside it.
pub type Strip<'a> = (&'a [TabLine], usize, Option<usize>);

/// A tab pressed (and maybe being dragged): its id, where the pointer holds it
/// (from its left edge), where the press was, and whether it moved yet.
pub struct TabDrag {
    id: u64,
    grab: f32,
    start_x: f32,
    moved: bool,
}

impl View {
    /// Tab keys, taken out of this frame's keys before the rest see them.
    pub(super) fn tab_keys(&mut self, ui: &mut Ui, tabs: &mut Tabs) {
        if self.keyboard_taken() || self.pick_typing(ui) {
            return;
        }
        let all = std::mem::take(&mut ui.s.input.keys);
        let before = tabs.pair_ids();
        let rest = all.into_iter().filter(|k| !self.tab_key(tabs, k)).collect();
        ui.s.input.keys = rest;
        if tabs.pair_ids() != before {
            ui.redraw();
        }
    }

    fn tab_key(&mut self, tabs: &mut Tabs, k: &Key) -> bool {
        let (ctrl, split) = (k.mods.ctrl, tabs.other().is_some());
        match (keys::ctrl_letter(k), k.sym) {
            (Some(b't'), _) => tabs.open(tabs.cur().cwd.clone(), true),
            (Some(b'w'), _) => {
                if !tabs.close(tabs.index()) {
                    self.quit = true;
                }
            }
            (_, KEY_F3) => tabs.unsplit(),
            (_, KEY_F5 | KEY_F6) if split => self.send_to_other(tabs, k.sym == KEY_F5),
            (_, KEY_TAB) if split && !ctrl => tabs.focus_other(),
            (_, KEY_PAGE_UP) if ctrl => tabs.step(-1),
            (_, KEY_PAGE_DOWN) if ctrl => tabs.step(1),
            _ => return false,
        }
        true
    }

    /// A middle click (last frame's layout): on a tab shows it beside this one, on a
    /// folder or a place opens it in a tab behind this one.
    pub(super) fn middle(&mut self, tabs: &Tabs, pos: (f32, f32)) {
        if let Some(i) = self.tab_rects.iter().position(|(r, _)| r.contains(pos)) {
            self.tab_reqs.push(TabReq::Split(i));
            return;
        }
        let b = tabs.cur();
        let path = self.places.at(pos).map(|t| t.1).or_else(|| {
            let e = &b.entries[self.item_at(pos, b.entries.len())?];
            e.dir.then(|| b.cwd.join(&e.os))
        });
        if let Some(p) = path {
            self.tab_reqs.push(TabReq::Open(p, false));
        }
    }

    /// Do the tab changes asked for this frame. True if any.
    pub(super) fn apply_tab_reqs(&mut self, tabs: &mut Tabs) -> bool {
        let any = !self.tab_reqs.is_empty();
        for r in std::mem::take(&mut self.tab_reqs) {
            match r {
                TabReq::Open(p, show) => tabs.open(p, show),
                TabReq::Close(i) => {
                    tabs.close(i);
                }
                TabReq::Show(i) => tabs.show(i),
                TabReq::Split(i) => tabs.split_with(i),
                TabReq::Move(from, to) => tabs.move_tab(from, to),
            }
        }
        any
    }

    /// The strip at the top of `panel` (when 2+ tabs), and the panel left below it.
    pub(super) fn tab_strip(
        &mut self,
        ui: &mut Ui,
        panel: Rect,
        (tabs, cur, other): Strip,
    ) -> Rect {
        let want = if tabs.len() > 1 { TAB_H + S4 } else { 0.0 };
        let h = ui.spring_from("tabs/h", 0.0, want, GLIDE).round();
        // Tabs on a strip that was hidden come in full size; later ones grow in.
        let fresh = self.tab_rects.is_empty();
        if h < 1.0 {
            (self.tab_drag, self.tab_rects) = (None, Vec::new());
            return panel;
        }
        let n = tabs.len() as f32;
        let w = ((panel.w - (n - 1.0) * S4) / n).min(TAB_MAX_W).floor();
        let slot = |i: usize| panel.x + i as f32 * (w + S4);
        self.drag_tab(ui, tabs, (slot(0), w));
        self.tab_rects.clear();
        ui.p.clip(Some(Rect { h, ..panel }));
        let dragged = self.tab_drag.as_ref().filter(|d| d.moved).map(|d| d.id);
        let mut on_top = None;
        for (i, (id, label, path)) in tabs.iter().enumerate() {
            let x_name = format!("tab/x/{id}");
            let x = match (&self.tab_drag, dragged == Some(*id)) {
                (Some(d), true) => {
                    // Follows the hand; the spring picks up from here on release.
                    let x = (ui.s.input.pos.0 - d.grab).clamp(panel.x, panel.x + panel.w - w);
                    ui.forget(&x_name);
                    ui.spring_from(&x_name, x, x, GLIDE)
                }
                _ => ui.spring(&x_name, slot(i), GLIDE),
            };
            let from = if fresh { w } else { 0.0 };
            let tw = ui.spring_from(&format!("tab/w/{id}"), from, w, GLIDE);
            let r = Rect::new(x, panel.y, tw, TAB_H).round();
            self.tab_rects.push((r, path.clone()));
            let state = (i, i == cur, Some(i) == other);
            if dragged == Some(*id) {
                on_top = Some((r, *id, label, state));
            } else {
                self.tab(ui, r, (*id, label), state);
            }
        }
        if let Some((r, id, label, state)) = on_top {
            self.tab(ui, r, (id, label), state);
        }
        ui.p.clip(None);
        Rect {
            y: panel.y + h,
            h: panel.h - h,
            ..panel
        }
    }

    /// Press, drag and release on the strip (tabs `w` wide, the first at `x0`): a
    /// release without moving shows the tab; dragging moves it to the slot it is over.
    fn drag_tab(&mut self, ui: &Ui, tabs: &[TabLine], (x0, w): (f32, f32)) {
        let i = &ui.s.input;
        if i.pressed && self.tab_drag.is_none() {
            let hit = self
                .tab_rects
                .iter()
                .zip(tabs)
                .find(|((r, _), _)| r.contains(i.pos));
            if let Some(((r, _), (id, ..))) = hit {
                let on_close = close_rect(*r).contains(i.pos);
                self.tab_drag = (!on_close).then_some(TabDrag {
                    id: *id,
                    grab: i.pos.0 - r.x,
                    start_x: i.pos.0,
                    moved: false,
                });
            }
        }
        let Some(d) = self.tab_drag.as_mut() else {
            return;
        };
        let Some(at) = tabs.iter().position(|t| t.0 == d.id) else {
            self.tab_drag = None;
            return;
        };
        if !i.down {
            if !d.moved {
                self.tab_reqs.push(TabReq::Show(at));
            }
            self.tab_drag = None;
            return;
        }
        d.moved |= (i.pos.0 - d.start_x).abs() >= DRAG_PX;
        if d.moved {
            let k = ((i.pos.0 - d.grab - x0 + (w + S4) / 2.0) / (w + S4)).floor();
            let to = (k.max(0.0) as usize).min(tabs.len() - 1);
            if to != at {
                self.tab_reqs.push(TabReq::Move(at, to));
            }
        }
    }

    /// One tab: folder icon, name, × (fading in on the current or hovered one). Hover,
    /// press, current and "beside" each glide in.
    fn tab(
        &mut self,
        ui: &mut Ui,
        r: Rect,
        (id, label): (u64, &str),
        (i, current, beside): (usize, bool, bool),
    ) {
        let held = self.tab_drag.as_ref().is_some_and(|d| d.id == id);
        let over = ui.over(r);
        ui.s.pointer |= over;
        let hover = ui.spring(&format!("tab/hover/{id}"), f32::from(over), SNAP);
        let cur = ui.spring(&format!("tab/cur/{id}"), f32::from(current), SNAP);
        let press = ui.spring(&format!("tab/press/{id}"), f32::from(held), SNAP);
        let side = ui.spring(&format!("tab/side/{id}"), f32::from(beside), SNAP);
        let pal = ui.pal;
        let fill = mix(mix(0, pal.hover, hover * 0.6), pal.raise, cur);
        ui.p.rect(r, RADIUS, mix(fill, pal.selected, press * 0.6));
        if side > 0.0 {
            ui.p.stroke(r, RADIUS, 1.0, mix(0, pal.sub, side * 0.5));
        }
        let color = mix(pal.sub, pal.text, cur.max(hover * 0.6));
        let icon_r = Rect::new(r.x + S8, r.y, ICON, r.h);
        ui.icon(Icon::FolderFill, icon_r, ICON, mix(pal.dim, color, 0.7));
        let tx = icon_r.x + ICON + S8;
        let close = close_rect(r);
        let ink = Ink::new(BODY, REGULAR, color);
        ui.text(
            Rect::new(tx, r.y, close.x - tx, r.h),
            label,
            ink,
            Align::Left,
        );
        let shown = cur.max(hover);
        if shown > 0.01 && self.close_button(ui, (id, close), shown) {
            self.tab_reqs.push(TabReq::Close(i));
        }
    }

    /// The ×: faded to `shown`, a round surface while hovered. True when clicked.
    fn close_button(&mut self, ui: &mut Ui, (id, r): (u64, Rect), shown: f32) -> bool {
        let name = format!("tab/close/{id}");
        let clicked = ui.clicked(&name, r);
        let over = ui.over(r);
        let hot = ui.spring(&format!("{name}/hover"), f32::from(over), SNAP);
        let pal = ui.pal;
        ui.p.rect(r, r.h / 2.0, mix(0, pal.selected, hot * shown));
        let color = mix(pal.sub, pal.text, hot);
        ui.icon(Icon::Close, r, ICON - 2.0, mix(0, color, shown));
        clicked
    }
}

/// Where a tab's × sits.
fn close_rect(r: Rect) -> Rect {
    Rect::new(
        r.x + r.w - CLOSE - S4,
        r.y + (r.h - CLOSE) / 2.0,
        CLOSE,
        CLOSE,
    )
    .round()
}

/// The strip's lines: each tab's id, the name of its folder, and the folder.
pub fn lines(tabs: &Tabs) -> Vec<TabLine> {
    tabs.each()
        .map(|(id, p)| {
            let name = if *p == trash::files_dir() {
                "Trash".into()
            } else if *p == super::home() {
                "Home".into()
            } else {
                p.file_name()
                    .map_or("/".into(), |n| n.to_string_lossy().into_owned())
            };
            (id, name, p.clone())
        })
        .collect()
}
