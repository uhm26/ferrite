//! Pixel work on an XRGB8888 canvas: fill, glyph blend, lines, cursor shapes.
//! Never reads the canvas, so it is safe on write-combined memory too.

pub struct Canvas<'a> {
    pub px: &'a mut [u32],
    pub w: usize,
    pub h: usize,
}

/// A pixel rectangle: x, y, width, height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// `fg` over `bg` at coverage `a` (0-255), per channel, rounded.
#[inline]
pub fn blend(bg: u32, fg: u32, a: u8) -> u32 {
    match a {
        0 => bg,
        255 => fg,
        _ => {
            let (a, ia) = (u32::from(a), 255 - u32::from(a));
            // All four channels: colors are premultiplied ARGB, so alpha blends alike.
            let ch = |s: u32| (((fg >> s & 0xff) * a + (bg >> s & 0xff) * ia + 127) / 255) << s;
            ch(24) | ch(16) | ch(8) | ch(0)
        }
    }
}

impl Canvas<'_> {
    pub fn fill(&mut self, r: Rect, color: u32) {
        let (x1, y1) = ((r.x + r.w).min(self.w), (r.y + r.h).min(self.h));
        if r.x >= x1 {
            return;
        }
        for y in r.y..y1 {
            self.px[y * self.w + r.x..y * self.w + x1].fill(color);
        }
    }

    /// Cell background with a coverage box blended on top in one pass.
    pub fn glyph(&mut self, r: Rect, cov: &[u8], bg: u32, fg: u32) {
        if cov.len() != r.w * r.h {
            return self.fill(r, bg);
        }
        let w = r.w.min(self.w.saturating_sub(r.x));
        for (gy, y) in (r.y..(r.y + r.h).min(self.h)).enumerate() {
            let line = &mut self.px[y * self.w + r.x..][..w];
            let src = &cov[gy * r.w..][..w];
            for (p, &a) in line.iter_mut().zip(src) {
                *p = blend(bg, fg, a);
            }
        }
    }

    /// Fill square `sq` by distance to circle `c` of radius `r`: `inside` color within
    /// the circle, `outside` beyond it, blended over one pixel at the edge.
    pub fn round(&mut self, sq: Rect, c: (f32, f32), r: f32, (inside, outside): (u32, u32)) {
        for y in sq.y..(sq.y + sq.h).min(self.h) {
            for x in sq.x..(sq.x + sq.w).min(self.w) {
                let (dx, dy) = (x as f32 + 0.5 - c.0, y as f32 + 0.5 - c.1);
                let cov = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                self.px[y * self.w + x] = blend(outside, inside, (cov * 255.0) as u8);
            }
        }
    }

    /// A rounded top corner (circle `c`, radius `r`) whose top edge carries an accent
    /// line `t` px thick at the top, tapering to nothing at the side, so the line
    /// follows the curve instead of stopping where it starts.
    pub fn capped_corner(
        &mut self,
        sq: Rect,
        c: (f32, f32),
        (r, t): (f32, f32),
        (tab, accent, bar): (u32, u32, u32),
    ) {
        for y in sq.y..(sq.y + sq.h).min(self.h) {
            for x in sq.x..(sq.x + sq.w).min(self.w) {
                let (dx, dy) = (x as f32 + 0.5 - c.0, y as f32 + 0.5 - c.1);
                let d = (dx * dx + dy * dy).sqrt().max(1e-3);
                let up = (-dy / d).max(0.0); // 1 straight up, 0 at the side
                let outer = (r - d + 0.5).clamp(0.0, 1.0);
                let inner = (r - t * up - d + 0.5).clamp(0.0, 1.0);
                let inside = blend(accent, tab, (inner * 255.0) as u8);
                self.px[y * self.w + x] = blend(bar, inside, (outer * 255.0) as u8);
            }
        }
    }

    /// Outline only (unfocused block cursor).
    pub fn frame(&mut self, r: Rect, t: usize, color: u32) {
        self.fill(Rect { h: t, ..r }, color);
        self.fill(
            Rect {
                y: r.y + r.h - t,
                h: t,
                ..r
            },
            color,
        );
        self.fill(Rect { w: t, ..r }, color);
        self.fill(
            Rect {
                x: r.x + r.w - t,
                w: t,
                ..r
            },
            color,
        );
    }

    /// Wavy underline: a sine, one period per cell, `t` px thick.
    pub fn curly(&mut self, r: Rect, cell_w: usize, t: usize, color: u32) {
        let amp = (r.h as f32 - t as f32).max(0.0) / 2.0;
        for x in r.x..(r.x + r.w).min(self.w) {
            let phase = (x - r.x) as f32 / cell_w as f32 * std::f32::consts::TAU;
            let dy = (amp * (1.0 - phase.sin())).round() as usize;
            self.fill(
                Rect {
                    x,
                    y: r.y + dy,
                    w: 1,
                    h: t,
                },
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_ends_and_middle() {
        assert_eq!(blend(0x000000, 0xffffff, 0), 0x000000);
        assert_eq!(blend(0x000000, 0xffffff, 255), 0xffffff);
        assert_eq!(blend(0x000000, 0xff0000, 128), 0x800000);
    }

    #[test]
    fn fill_clips() {
        let mut px = vec![0u32; 4 * 2];
        let mut c = Canvas {
            px: &mut px,
            w: 4,
            h: 2,
        };
        c.fill(
            Rect {
                x: 3,
                y: 1,
                w: 5,
                h: 5,
            },
            9,
        );
        assert_eq!(px, [0, 0, 0, 0, 0, 0, 0, 9]);
    }
}
