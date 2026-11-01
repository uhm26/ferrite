//! The folder as a grid of tiles on one panel (Nemo's icon view): a large icon,
//! the name under it. Tiles stretch to fill each row exactly; only rows in view
//! are drawn.

use frame::ui::paint::mix;
use frame::ui::theme::*;
use frame::ui::{Align, Rect, Ui};

use super::scroll::Scroll;
use crate::browse::Browser;

/// Narrowest tile; the row's tiles share any extra width.
const TILE_W: f32 = 104.0;
pub const TILE_H: f32 = 92.0;
/// Room for the icon at the top of a tile.
const ICON_BOX: f32 = 56.0;
/// Name line under the icon.
const NAME_H: f32 = 20.0;

#[derive(Default)]
pub struct Grid {
    /// Tiles per row last frame (↑/↓ move by this many).
    pub cols: usize,
}

impl Grid {
    /// Draw the grid in `panel`; returns the entry clicked, if any.
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        b: &Browser,
        panel: Rect,
        scroll: &mut Scroll,
    ) -> Option<usize> {
        ui.panel(panel);
        let inner = panel.inset(S8);
        if b.entries.is_empty() {
            super::empty(ui, inner);
            return None;
        }
        let cols = ((inner.w / TILE_W).floor() as usize).max(1);
        self.cols = cols;
        let tile_w = (inner.w / cols as f32).floor();
        let rows = b.entries.len().div_ceil(cols);
        let content = rows as f32 * TILE_H;
        let key = format!("scroll/grid/{}", b.cwd.display());
        let sel = ((b.sel / cols) as f32 * TILE_H, TILE_H);
        let off = scroll.offset(ui, &key, sel, (inner.h, content));
        let first = (off / TILE_H) as usize * cols;
        let last = ((((off + inner.h) / TILE_H).ceil() as usize) * cols).min(b.entries.len());
        let mut clicked = None;
        ui.p.clip(Some(inner));
        for i in first..last {
            let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
            let t = Rect::new(
                inner.x + cx * tile_w,
                inner.y + cy * TILE_H - off,
                tile_w,
                TILE_H,
            );
            if tile(ui, b, i, t.inset(2.0)) {
                clicked = Some(i);
            }
        }
        ui.p.clip(None);
        let over = panel.contains(ui.s.input.pos);
        ui.scroll_thumb(
            "scroll/grid/thumb",
            panel.inset(S4),
            (off, inner.h, content),
            over,
        );
        clicked
    }
}

/// One tile: frame's row for hover and selection, a 32 px icon, the name cut to fit.
fn tile(ui: &mut Ui, b: &Browser, i: usize, t: Rect) -> bool {
    let id = format!("tile/{i}");
    let clicked = ui.row(&id, t, i == b.sel);
    let sel = ui.spring(&id, f32::from(i == b.sel), SNAP);
    let e = &b.entries[i];
    let color = mix(ui.pal.sub, ui.pal.text, sel);
    let icon_r = Rect::new(t.x, t.y + S8, t.w, ICON_BOX - S8);
    ui.icon(super::icon(e), icon_r, 2.0 * ICON, color);
    let name = Rect::new(t.x + S8, t.y + ICON_BOX + S4, t.w - 2.0 * S8, NAME_H);
    let body = ui.body();
    ui.text(name, &e.name, body, Align::Center);
    clicked
}
