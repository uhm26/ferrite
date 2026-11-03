//! One font face at one size: [`ttf`] reads the file, [`raster`] draws the glyphs.
//! The same calls the FreeType wrapper had, so text code does not change.

use std::os::fd::AsFd;
use std::path::Path;

use super::raster::{self, Bitmap};
use super::ttf;

/// A drawn glyph: `rows` lines of `pitch` bytes of coverage (0–255). `left`, `top`:
/// offset of its top-left pixel from the pen (x right, y up from the baseline).
pub struct Rendered<'a> {
    pub bitmap: &'a [u8],
    pub width: usize,
    pub rows: usize,
    pub pitch: usize,
    pub left: i32,
    pub top: i32,
}

pub struct Face {
    font: ttf::Font,
    /// Pixels per font unit.
    scale: f32,
    /// Pixels per font unit up and down: a touch off `scale` so the x-height lands on
    /// a whole pixel, which keeps the tops of lowercase letters crisp (the main thing
    /// FreeType's light hinting does).
    y_scale: f32,
    /// Height of 'x', font units (0: no x, no snapping).
    x_height: f32,
    /// The last glyph drawn; [`Rendered`] borrows it.
    last: Bitmap,
}

/// Whole px for an x-height: up from .375 (24/64) like FreeType's autohinter, so
/// small text keeps its body instead of shrinking.
fn snap(px: f32) -> f32 {
    let floor = px.floor();
    (if px - floor >= 0.375 { floor + 1.0 } else { floor }).max(1.0)
}

impl Face {
    /// Face `index` of the file at `path` (0 unless it is a collection). None for
    /// files this reader can't use (CFF outlines, bitmap-only emoji fonts).
    pub fn open(path: &Path, index: i32) -> Option<Face> {
        // Mapped, not read: a CJK fallback font is ~20 MB and we touch a few glyphs.
        let file = std::fs::File::open(path).ok()?;
        let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
        let map = crate::sys::mem::Map::read_only(file.as_fd(), len).ok()?;
        let font = ttf::Font::parse(ttf::Bytes::Mapped(map), u32::try_from(index).ok()?).ok()?;
        let x_height = f32::from(font.top(font.glyph('x')).max(0));
        let last = raster::rasterize(&[], (1.0, 1.0), 0.0);
        let mut face = Face { font, scale: 0.0, y_scale: 0.0, x_height, last };
        face.set_size(12.0);
        Some(face)
    }

    /// Size in points at 96 dpi. Not snapped to whole pixels per em: FreeType's light
    /// hinting doesn't either, and cell sizes must stay the same as before.
    pub fn set_size(&mut self, pt: f32) -> bool {
        self.scale = pt * 96.0 / 72.0 / f32::from(self.font.units_per_em);
        let xh = self.x_height * self.scale;
        self.y_scale = if xh > 0.0 { snap(xh) / self.x_height } else { self.scale };
        true
    }

    pub fn has(&self, c: char) -> bool {
        self.font.glyph(c) != 0
    }

    /// Line metrics in 26.6 fixed point: ascender (rounded up to whole px),
    /// descender (negative, rounded down).
    pub fn metrics(&self) -> (i64, i64) {
        let px = |v: i16, f: fn(f32) -> f32| f(f32::from(v) * self.scale) as i64 * 64;
        (
            px(self.font.ascender, f32::ceil),
            px(self.font.descender, f32::floor),
        )
    }

    /// Draw `c`. Advance in 26.6, rounded to whole px (cell grids want that).
    pub fn render(&mut self, c: char) -> Option<(Rendered<'_>, i64)> {
        self.render_at(c, 0).map(|(r, adv, _)| (r, adv))
    }

    /// [`Face::render`] shifted right by `dx` (26.6, a subpixel phase), plus the
    /// exact advance in px for subpixel-positioned text.
    pub fn render_at(&mut self, c: char, dx: i64) -> Option<(Rendered<'_>, i64, f32)> {
        let g = self.font.glyph(c);
        let tol = raster::TOLERANCE / self.scale.max(1e-6); // px → font units
        let outline = self.font.outline(g, tol).ok()?;
        self.last = raster::rasterize(&outline, (self.scale, self.y_scale), dx as f32 / 64.0);
        let linear = self.font.advance(g) * self.scale;
        let b = &self.last;
        let r = Rendered {
            bitmap: &b.px,
            width: b.width,
            rows: b.rows,
            pitch: b.width,
            left: b.left,
            top: b.top,
        };
        Some((r, linear.round() as i64 * 64, linear))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adwaita Mono (installed with the adwaita-fonts package, also in CI): metrics,
    /// advance and pixels as FreeType gave them before the switch (11 pt).
    #[test]
    fn draws_a_real_font_like_freetype_did() {
        let (path, index) = super::super::fc::find("Adwaita Mono", None).unwrap();
        let mut f = Face::open(&path, index).unwrap();
        f.set_size(11.0);
        assert_eq!(f.metrics(), (960, -320));
        let (r, adv) = f.render('M').unwrap();
        assert_eq!(adv, 576); // 9 px cells
        assert!(r.width > 4 && r.rows > 8 && r.bitmap.contains(&255));
        let (space, _) = f.render(' ').unwrap();
        assert_eq!(space.width, 0);
        assert!(f.has('ä') && !f.has('\u{e000}'));
    }
}
