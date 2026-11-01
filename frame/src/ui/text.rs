//! Text: glyphs rasterized once per (char, weight, px size, ¼-px phase) and kept.
//! The pen moves by unhinted advances, so letter spacing is even, and each glyph is
//! rendered at the quarter pixel where it lands (boltsnap does the same).
//! Baselines sit on whole px, so vertical stems stay crisp. Boxes center text by
//! cap height (resize.md → Text), not by em.

use std::borrow::Cow;
use std::collections::HashMap;

use super::paint::Painter;
use crate::font::{self, Font};

/// Weights, index with these: what `Ink::style` holds.
pub const REGULAR: usize = 0;
pub const MEDIUM: usize = 1;
pub const SEMIBOLD: usize = 2;
pub const BOLD: usize = 3;
/// fontconfig suffixes for the weights above, in order.
const WEIGHTS: [&str; 4] = ["", ":medium", ":semibold", ":bold"];
/// Subpixel positions per px.
const PHASES: f32 = 4.0;
const ELLIPSIS: &str = "…";

struct Glyph {
    left: i32,
    top: i32,
    w: usize,
    h: usize,
    cov: Box<[u8]>,
    /// Unhinted, px.
    advance: f32,
}

/// How text looks: px size, weight ([`REGULAR`]…) and color.
#[derive(Clone, Copy, Debug)]
pub struct Ink {
    pub px: f32,
    pub style: usize,
    pub color: u32,
}

impl Ink {
    pub const fn new(px: f32, style: usize, color: u32) -> Self {
        Ink { px, style, color }
    }

    pub const fn color(self, color: u32) -> Self {
        Ink { color, ..self }
    }
}

/// Ascent, descent and cap height in px at one size.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub ascent: f32,
    pub descent: f32,
    pub cap: f32,
}

pub struct Text {
    font: Font,
    /// The px size the font is set to now.
    at: u32,
    glyphs: HashMap<(char, u8, u32, u8), Glyph>,
    metrics: HashMap<(u8, u32), Metrics>,
}

/// Sizes key the caches in 1/4 px.
fn key(px: f32) -> u32 {
    (px * 4.0).round() as u32
}

impl Text {
    /// `family` in the four weights; fontconfig picks the nearest face for each.
    pub fn new(family: &str) -> Result<Self, font::Error> {
        Ok(Text {
            font: Font::with_styles(family, 12.0, &WEIGHTS)?,
            at: 0,
            glyphs: HashMap::new(),
            metrics: HashMap::new(),
        })
    }

    /// Point the font at `px` (FreeType sizes in pt at 96 dpi: pt = px · 0.75).
    fn size(&mut self, k: u32) {
        if self.at != k {
            self.font.resize(k as f32 / 4.0 * 0.75);
            self.at = k;
        }
    }

    fn glyph(&mut self, c: char, style: usize, k: u32, phase: u8) -> &Glyph {
        let id = (c, style as u8, k, phase);
        if !self.glyphs.contains_key(&id) {
            self.size(k);
            // A quarter px is 16 in 26.6.
            let dx = i64::from(phase) * 16;
            let g = match self.font.face_for(c, style).render_at(c, dx) {
                Some((r, _, advance)) => Glyph {
                    left: r.left,
                    top: r.top,
                    w: r.width,
                    h: r.rows,
                    cov: (0..r.rows)
                        .flat_map(|y| &r.bitmap[y * r.pitch..y * r.pitch + r.width])
                        .copied()
                        .collect(),
                    advance,
                },
                None => Glyph {
                    left: 0,
                    top: 0,
                    w: 0,
                    h: 0,
                    cov: Box::new([]),
                    advance: 0.0,
                },
            };
            self.glyphs.insert(id, g);
        }
        &self.glyphs[&id]
    }

    pub fn metrics(&mut self, px: f32, style: usize) -> Metrics {
        let k = key(px);
        if let Some(m) = self.metrics.get(&(style as u8, k)) {
            return *m;
        }
        self.size(k);
        let (asc, desc) = self.font.face(style).metrics();
        let cap = self.glyph('H', style, k, 0).top as f32;
        let m = Metrics {
            ascent: asc as f32 / 64.0,
            descent: -desc as f32 / 64.0,
            cap,
        };
        self.metrics.insert((style as u8, k), m);
        m
    }

    /// Width of `s` in px (fractional: the pen's travel).
    pub fn width(&mut self, s: &str, ink: Ink) -> f32 {
        let k = key(ink.px);
        s.chars()
            .map(|c| self.glyph(c, ink.style, k, 0).advance)
            .sum()
    }

    /// `s` cut with `…` so it fits `max` px; unchanged if it already fits.
    pub fn ellipsize<'s>(&mut self, s: &'s str, max: f32, ink: Ink) -> Cow<'s, str> {
        if self.width(s, ink) <= max {
            return Cow::Borrowed(s);
        }
        let room = max - self.width(ELLIPSIS, ink);
        let k = key(ink.px);
        let mut used = 0.0;
        let mut end = 0;
        for (i, c) in s.char_indices() {
            used += self.glyph(c, ink.style, k, 0).advance;
            if used > room {
                break;
            }
            end = i + c.len_utf8();
        }
        Cow::Owned(format!("{}{ELLIPSIS}", s[..end].trim_end()))
    }

    /// Draw `s` with its baseline at `(x, y)`. Returns the pen's x after it.
    pub fn draw(&mut self, p: &mut Painter, (x, y): (f32, f32), s: &str, ink: Ink) -> f32 {
        let k = key(ink.px);
        let base = y.round() as i32;
        let mut pen = x;
        for c in s.chars() {
            let whole = pen.floor();
            let phase = (((pen - whole) * PHASES) as u8).min(PHASES as u8 - 1);
            let g = self.glyph(c, ink.style, k, phase);
            if g.w > 0 {
                let at = (whole as i32 + g.left, base - g.top);
                p.coverage(at, (g.w, g.h), &g.cov, ink.color);
            }
            pen += g.advance;
        }
        pen
    }

    /// Baseline y that centers capitals in a box from `top`, `h` high.
    pub fn center(&mut self, top: f32, h: f32, ink: Ink) -> f32 {
        let cap = self.metrics(ink.px, ink.style).cap;
        (top + (h + cap) / 2.0).round()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::FAMILY;

    #[test]
    fn widths_are_fractional_and_additive() {
        let mut t = Text::new(FAMILY).unwrap();
        let ink = Ink::new(14.0, MEDIUM, 0);
        let (a, b) = (t.width("files", ink), t.width("frame", ink));
        assert!((t.width("filesframe", ink) - (a + b)).abs() < 0.01);
        assert!(a > 20.0 && a < 60.0, "{a}");
    }

    #[test]
    fn ellipsize_fits_and_keeps_short_text() {
        let mut t = Text::new(FAMILY).unwrap();
        let ink = Ink::new(14.0, REGULAR, 0);
        assert_eq!(t.ellipsize("ok", 100.0, ink), "ok");
        let long = "a-very-long-file-name-that-does-not-fit.tar.gz";
        let cut = t.ellipsize(long, 80.0, ink);
        assert!(cut.ends_with('…'), "{cut}");
        assert!(t.width(&cut, ink) <= 80.0);
    }

    #[test]
    fn weights_differ() {
        let mut t = Text::new(FAMILY).unwrap();
        let w = |t: &mut Text, s| t.width("Documents", Ink::new(14.0, s, 0));
        assert!(w(&mut t, BOLD) > w(&mut t, REGULAR));
    }
}
