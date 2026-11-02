//! The entries: a list (list.rs) or a grid of tiles (grid.rs), scrolled (scroll.rs),
//! and what the pointer does on them besides clicking (drag.rs): the selection box,
//! and dragging entries onto folders or out to other apps.

pub mod drag;
pub mod grid;
pub mod list;
pub mod panes;
pub mod scroll;

use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, Rect, Ui};

use super::{LIST, View};

impl View {
    /// Entry `i`'s row (or tile) and name cell, from last frame's layout.
    pub(super) fn item_rects(&self, i: usize) -> Option<(Rect, Rect)> {
        if self.mode == LIST {
            self.list.rects(i)
        } else {
            self.grid.rects(i)
        }
    }

    /// The entry under `pos` (of `n`), from last frame's layout.
    pub(super) fn item_at(&self, pos: (f32, f32), n: usize) -> Option<usize> {
        if self.mode == LIST {
            self.list.at(pos, n)
        } else {
            self.grid.at(pos, n)
        }
    }
}

/// "This folder is empty", centered in `r` under a large folder.
fn empty(ui: &mut Ui, r: Rect) {
    let (sub, dim) = (ui.pal.sub, ui.pal.dim);
    let mid = r.y + (r.h / 2.0).round();
    ui.icon(
        Icon::Folder,
        Rect::new(r.x, mid - 48.0, r.w, 40.0),
        2.0 * ICON,
        dim,
    );
    let ink = Ink::new(BODY, REGULAR, sub);
    ui.text(
        Rect::new(r.x, mid, r.w, 20.0),
        "This folder is empty",
        ink,
        Align::Center,
    );
}
