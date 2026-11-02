//! The folder as a grid of tiles straight on the window (Nemo's icon view): a large icon
//! (or the picture itself, see thumbs.rs), the name under it. Tiles stretch to fill
//! each row exactly; only rows in view are drawn. Ctrl+wheel or Ctrl+± zoom through
//! four tile sizes.

use frame::ui::theme::*;
use frame::ui::{Align, Rect, Ui};

use super::scroll::Scroll;
use crate::browse::Browser;
use crate::dir::{self, Kind};
use crate::thumbs::Thumbs;

/// Per zoom level: narrowest tile (the row's tiles share any extra width), the box
/// the icon or picture sits in at the top of a tile, and the icon's size.
const LEVELS: [(f32, f32, f32); 4] = [
    (104.0, 56.0, 2.0 * ICON),
    (128.0, 80.0, 3.0 * ICON),
    (160.0, 112.0, 4.0 * ICON),
    (208.0, 160.0, 6.0 * ICON),
];
/// Name line under the icon, and the room around it.
const NAME_H: f32 = 20.0;
const UNDER_ICON: f32 = NAME_H + 16.0;
/// Pictures get slightly rounded corners.
const PIC_RADIUS: f32 = 4.0;

#[derive(Default)]
pub struct Grid {
    /// Tiles per row last frame (↑/↓ move by this many).
    pub cols: usize,
    /// Prefix for its widget names (see `List::tag`).
    pub tag: &'static str,
    /// Drawn as the pane not in use: no selection shown.
    pub passive: bool,
    /// Index into [`LEVELS`].
    pub zoom: usize,
    /// Last frame's area, scroll offset and tile width (right-click target,
    /// menu key, rename field).
    geo: Option<(Rect, f32, f32)>,
}

impl Grid {
    pub fn tile_h(&self) -> f32 {
        LEVELS[self.zoom].1 + UNDER_ICON
    }

    /// Bigger (`d` > 0) or smaller tiles. False at either end.
    pub fn zoom_by(&mut self, d: i32) -> bool {
        let z = (self.zoom as i32 + d.signum()).clamp(0, LEVELS.len() as i32 - 1) as usize;
        std::mem::replace(&mut self.zoom, z) != z
    }

    /// The entry under `pos`, from last frame's layout.
    pub fn at(&self, pos: (f32, f32), n: usize) -> Option<usize> {
        let (inner, off, tile_w) = self.geo?;
        let col = ((pos.0 - inner.x) / tile_w).floor() as usize;
        let row = ((pos.1 - inner.y + off) / self.tile_h()).floor() as usize;
        let i = row * self.cols + col;
        (inner.contains(pos) && col < self.cols && i < n).then_some(i)
    }

    /// Entry `i`'s tile and its name line, if on screen last frame.
    pub fn rects(&self, i: usize) -> Option<(Rect, Rect)> {
        let (inner, off, tile_w) = self.geo?;
        let t = self.tile_rect(inner, off, tile_w, i);
        let visible = t.y >= inner.y - 1.0 && t.y + t.h <= inner.y + inner.h + 1.0;
        let top = LEVELS[self.zoom].1;
        let name = Rect::new(t.x + S4, t.y + top, t.w - 2.0 * S4, NAME_H + S8);
        visible.then_some((t, name))
    }

    fn tile_rect(&self, inner: Rect, off: f32, tile_w: f32, i: usize) -> Rect {
        let cols = self.cols.max(1);
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
        let h = self.tile_h();
        Rect::new(inner.x + cx * tile_w, inner.y + cy * h - off, tile_w, h)
    }

    /// Draw the grid in `panel`; returns the entry clicked, if any.
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        b: &Browser,
        (panel, thumbs): (Rect, &mut Thumbs),
        scroll: &mut Scroll,
    ) -> Option<usize> {
        let inner = panel;
        self.geo = None;
        if b.entries.is_empty() {
            super::empty(ui, inner);
            return None;
        }
        let (min_w, tile_h) = (LEVELS[self.zoom].0, self.tile_h());
        let cols = ((inner.w / min_w).floor() as usize).max(1);
        self.cols = cols;
        let tile_w = (inner.w / cols as f32).floor();
        let rows = b.entries.len().div_ceil(cols);
        let content = rows as f32 * tile_h;
        let key = format!("{}scroll/grid/{}", self.tag, b.cwd.display());
        let sel = ((b.sel / cols) as f32 * tile_h, tile_h);
        let off = scroll.offset(ui, &key, sel, (inner.h, content));
        self.geo = Some((inner, off, tile_w));
        let first = (off / tile_h) as usize * cols;
        let last = ((((off + inner.h) / tile_h).ceil() as usize) * cols).min(b.entries.len());
        let mut clicked = None;
        ui.p.clip(Some(inner));
        for i in first..last {
            let t = self.tile_rect(inner, off, tile_w, i);
            if tile(ui, b, ((self.tag, self.passive), i, t.inset(2.0)), (self.zoom, thumbs)) {
                clicked = Some(i);
            }
        }
        ui.p.clip(None);
        let over = panel.contains(ui.s.input.pos);
        ui.scroll_thumb(
            &format!("{}scroll/grid/thumb", self.tag),
            panel.inset(S4),
            (off, inner.h, content),
            over,
        );
        clicked
    }
}

/// One tile: frame's row for hover and selection, the icon or the picture, the name
/// cut to fit.
fn tile(
    ui: &mut Ui,
    b: &Browser,
    ((tag, passive), i, t): ((&str, bool), usize, Rect),
    (zoom, thumbs): (usize, &mut Thumbs),
) -> bool {
    let id = format!("{tag}tile/{i}");
    let on = !passive && b.is_selected(i);
    let clicked = ui.row(&id, t, on);
    let sel = ui.spring(&id, f32::from(on), SNAP);
    let e = &b.entries[i];
    let (_, top, glyph) = LEVELS[zoom];
    let icon_r = Rect::new(t.x, t.y + S8, t.w, top - S8);
    let path = b.cwd.join(&e.os);
    let side = (icon_r.h - S4) as usize;
    let pic = (dir::kind(e).0 == Kind::Image)
        .then(|| thumbs.get(&path, e.modified, side))
        .flatten();
    match pic {
        Some(p) => {
            // Fades in once made, instead of popping over the icon.
            let a = ui.spring_from(&format!("thumb/{}", path.display()), 0.0, 1.0, SNAP);
            let x = icon_r.x + ((icon_r.w - p.w as f32) / 2.0).round();
            let y = icon_r.y + ((icon_r.h - p.h as f32) / 2.0).round();
            ui.p.image((x as i32, y as i32), (p.w, p.h), &p.px, (PIC_RADIUS, a));
        }
        None => {
            let color = crate::view::icon_color(ui, e, sel);
            ui.icon(crate::view::icon(e), icon_r, glyph, color);
        }
    }
    let name = Rect::new(t.x + S8, t.y + top + S4, t.w - 2.0 * S8, NAME_H);
    let body = ui.body();
    ui.text(name, &e.name, body, Align::Center);
    clicked
}

impl Grid {
    /// One whose widget names start with `tag` (the split view's second pane).
    pub fn tagged(tag: &'static str) -> Self {
        Grid {
            tag,
            ..Grid::default()
        }
    }
}
