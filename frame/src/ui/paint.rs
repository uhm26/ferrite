//! Painting on a window canvas: rounded rects, hairline strokes, soft shadows and
//! coverage bitmaps (glyphs), all antialiased from one signed distance function.
//! Colors are premultiplied ARGB; everything blends with "over".
//! `blend` started from ../term/src/render/draw.rs at term 7be66f5.

use crate::window::Canvas;

/// A rectangle in logical px (scale 1 today, so also device px).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, (px, py): (f32, f32)) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }

    /// Shrunk by `d` on every side (grown if negative).
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }

    /// Snapped to whole px, so edges never shimmer mid-animation.
    pub fn round(&self) -> Rect {
        let (x, y) = (self.x.round(), self.y.round());
        Rect::new(
            x,
            y,
            (self.x + self.w).round() - x,
            (self.y + self.h).round() - y,
        )
    }

    /// The point-wise mix of two rects (`t` 0 = self, 1 = other).
    pub fn lerp(&self, o: &Rect, t: f32) -> Rect {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Rect::new(
            l(self.x, o.x),
            l(self.y, o.y),
            l(self.w, o.w),
            l(self.h, o.h),
        )
    }
}

/// `src` (premultiplied) over `dst` at coverage `cov` (0-255).
#[inline]
pub fn over(dst: u32, src: u32, cov: u32) -> u32 {
    let a = ((src >> 24) * cov + 127) / 255;
    if a == 0 {
        return dst;
    }
    if a == 255 {
        return src;
    }
    let ia = 255 - a;
    let ch = |s: u32| {
        let sc = ((src >> s & 0xff) * cov + 127) / 255;
        (sc + ((dst >> s & 0xff) * ia + 127) / 255).min(255) << s
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Per-channel mix of two colors (`t` 0 = a, 1 = b).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = (t.clamp(0.0, 1.0) * 256.0) as u32;
    let ch = |s: u32| ((a >> s & 0xff) * (256 - t) + (b >> s & 0xff) * t) >> 8 << s;
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Signed distance from pixel center `(px, py)` to a rounded rect: < 0 inside.
#[inline]
pub(crate) fn sdf(r: &Rect, rad: f32, px: f32, py: f32) -> f32 {
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    let qx = (px - r.x - hw).abs() - (hw - rad);
    let qy = (py - r.y - hh).abs() - (hh - rad);
    let outside = if qx > 0.0 && qy > 0.0 {
        (qx * qx + qy * qy).sqrt()
    } else {
        qx.max(qy).max(0.0)
    };
    outside + qx.max(qy).min(0.0) - rad
}

/// Pixel columns `[a, b)` of row center `py` whose centers lie inside `r` shrunk by
/// `deep` (same center, radius `rad − deep`): every one of them has distance ≤ −deep.
fn inner_span(r: &Rect, rad: f32, deep: f32, py: f32) -> Option<(usize, usize)> {
    let (hw, hh, rr) = (r.w / 2.0 - deep, r.h / 2.0 - deep, (rad - deep).max(0.0));
    let ay = (py - r.y - r.h / 2.0).abs();
    if hw <= 0.0 || ay > hh {
        return None;
    }
    let dy = ay - (hh - rr);
    let half = if dy <= 0.0 {
        hw
    } else {
        hw - rr + (rr * rr - dy * dy).max(0.0).sqrt()
    };
    let cx = r.x + r.w / 2.0;
    let a = (cx - half - 0.5).ceil().max(0.0) as usize;
    let b = ((cx + half - 0.5).floor() + 1.0).max(0.0) as usize;
    Some((a, b))
}

pub struct Painter<'a> {
    px: &'a mut [u32],
    w: usize,
    h: usize,
    /// x0, y0, x1, y1 in px, inside the canvas.
    clip: [usize; 4],
}

impl<'a> Painter<'a> {
    pub fn new(c: Canvas<'a>) -> Self {
        let clip = [0, 0, c.w, c.h];
        Painter {
            px: c.px,
            w: c.w,
            h: c.h,
            clip,
        }
    }

    /// Whether `(x, y)` lies inside the current clip: clipped-away parts of a
    /// widget must not take the pointer either.
    pub fn in_clip(&self, (x, y): (f32, f32)) -> bool {
        let [x0, y0, x1, y1] = self.clip.map(|v| v as f32);
        x >= x0 && y >= y0 && x < x1 && y < y1
    }

    pub fn size(&self) -> (f32, f32) {
        (self.w as f32, self.h as f32)
    }

    /// Paint only inside `r` from now on (None: the whole canvas).
    pub fn clip(&mut self, r: Option<Rect>) {
        self.clip = match r {
            None => [0, 0, self.w, self.h],
            Some(r) => self.span(&r.round(), 0.0),
        };
    }

    /// Pixel bounds of `r` grown by `pad`, clipped.
    fn span(&self, r: &Rect, pad: f32) -> [usize; 4] {
        let c = self.clip;
        let lo = |v: f32, min: usize, max: usize| (v.floor().max(0.0) as usize).clamp(min, max);
        let x0 = lo(r.x - pad, c[0], c[2]);
        let y0 = lo(r.y - pad, c[1], c[3]);
        let x1 = lo((r.x + r.w + pad).ceil(), c[0], c[2]);
        let y1 = lo((r.y + r.h + pad).ceil(), c[1], c[3]);
        [x0, y0, x1, y1]
    }

    pub fn clear(&mut self, color: u32) {
        self.px.fill(color);
    }

    /// Every pixel of `r` grown by `pad`: `cov(distance)` 0-1 decides how much of
    /// `color` lands. Pixels at least `deep` px inside skip the distance math: they
    /// get `color` in full when `fill`, else nothing (most of a card's area).
    fn shape(
        &mut self,
        r: Rect,
        (rad, pad, deep): (f32, f32, f32),
        (color, fill): (u32, bool),
        cov: impl Fn(f32) -> f32,
    ) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return; // nothing to paint (tiny windows shrink rects below zero)
        }
        let rad = rad.clamp(0.0, r.w.min(r.h) / 2.0);
        let [x0, y0, x1, y1] = self.span(&r, pad);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let (ix0, ix1) = inner_span(&r, rad, deep, py).map_or((x1, x1), |(a, b)| {
                (a.clamp(x0, x1), b.clamp(x0, x1).max(a.clamp(x0, x1)))
            });
            let row = &mut self.px[y * self.w..][..x1];
            for x in (x0..ix0).chain(ix1..x1) {
                let c = (cov(sdf(&r, rad, x as f32 + 0.5, py)) * 255.0 + 0.5) as u32;
                if c > 0 {
                    row[x] = over(row[x], color, c.min(255));
                }
            }
            let inner = &mut row[ix0..ix1];
            match (fill, color >> 24) {
                (false, _) => {}
                (true, 0xff) => inner.fill(color),
                (true, _) => inner.iter_mut().for_each(|p| *p = over(*p, color, 255)),
            }
        }
    }

    /// A filled rounded rect.
    pub fn rect(&mut self, r: Rect, rad: f32, color: u32) {
        self.shape(r, (rad, 0.0, 1.0), (color, true), |d| {
            (0.5 - d).clamp(0.0, 1.0)
        });
    }

    /// An outline `width` px wide, just inside the edge of `r` (a hairline at 1).
    pub fn stroke(&mut self, r: Rect, rad: f32, width: f32, color: u32) {
        self.shape(r, (rad, 0.0, width + 1.0), (color, false), |d| {
            (0.5 - d).clamp(0.0, 1.0) - (0.5 - d - width).clamp(0.0, 1.0)
        });
    }

    /// Soft black shadow around `r`, outside it only (a glass card above stays clear).
    pub fn shadow(&mut self, r: Rect, rad: f32, blur: f32, alpha: f32) {
        let color = 0xff00_0000;
        self.shape(r, (rad, blur, 1.0), (color, false), |d| {
            if d < 0.0 {
                return 0.0;
            }
            let t = (d / blur).clamp(0.0, 1.0);
            alpha * (1.0 - t * t * (3.0 - 2.0 * t))
        });
    }

    /// Blend `color` through an 8-bit coverage bitmap whose top-left is at `(x, y)`.
    pub fn coverage(&mut self, (x, y): (i32, i32), (w, h): (usize, usize), cov: &[u8], color: u32) {
        let c = self.clip;
        for gy in 0..h {
            let py = y + gy as i32;
            if py < c[1] as i32 || py >= c[3] as i32 {
                continue;
            }
            for gx in 0..w {
                let px = x + gx as i32;
                let a = u32::from(cov[gy * w + gx]);
                if a != 0 && px >= c[0] as i32 && px < c[2] as i32 {
                    let p = &mut self.px[py as usize * self.w + px as usize];
                    *p = over(*p, color, a);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(px: &mut [u32], w: usize, h: usize) -> Painter<'_> {
        Painter::new(Canvas { px, w, h })
    }

    #[test]
    fn over_ends_and_half() {
        assert_eq!(over(0xff000000, 0xffffffff, 255), 0xffffffff);
        assert_eq!(over(0xff123456, 0xffffffff, 0), 0xff123456);
        // white at 50 % (premultiplied 0x80808080) over black
        assert_eq!(over(0xff000000, 0x80808080, 255), 0xff808080);
    }

    #[test]
    fn square_rect_is_crisp_and_clipped() {
        let mut px = vec![0xff000000; 8 * 8];
        let mut p = canvas(&mut px, 8, 8);
        p.rect(Rect::new(2.0, 2.0, 10.0, 3.0), 0.0, 0xffffffff);
        assert_eq!(px[2 * 8 + 1], 0xff000000); // left of it
        assert_eq!(px[2 * 8 + 2], 0xffffffff); // first pixel inside
        assert_eq!(px[4 * 8 + 7], 0xffffffff); // clipped at the canvas edge
        assert_eq!(px[5 * 8 + 4], 0xff000000); // below it
    }

    #[test]
    fn negative_rects_paint_nothing_and_never_panic() {
        let mut px = vec![0xff00_0000; 8 * 8];
        let mut p = canvas(&mut px, 8, 8);
        let r = Rect::new(2.0, 2.0, -3.0, 4.0);
        p.rect(r, 8.0, 0xffff_ffff);
        p.stroke(r, 8.0, 1.0, 0xffff_ffff);
        p.shadow(Rect::new(2.0, 2.0, 4.0, -1.0), 2.0, 4.0, 0.5);
        assert!(px.iter().all(|&c| c == 0xff00_0000));
    }

    #[test]
    fn hairline_is_one_px() {
        let mut px = vec![0xff000000; 6 * 6];
        let mut p = canvas(&mut px, 6, 6);
        p.stroke(Rect::new(0.0, 0.0, 6.0, 6.0), 0.0, 1.0, 0xffffffff);
        assert_eq!(px[0], 0xffffffff);
        assert_eq!(px[6 + 1], 0xff000000);
        assert_eq!(px[3 * 6 + 5], 0xffffffff);
    }

    #[test]
    fn rounded_corner_is_soft() {
        let mut px = vec![0xff000000; 16 * 16];
        let mut p = canvas(&mut px, 16, 16);
        p.rect(Rect::new(0.0, 0.0, 16.0, 16.0), 8.0, 0xffffffff);
        assert_eq!(px[0], 0xff000000); // corner pixel: outside the circle
        assert_eq!(px[8 * 16 + 8], 0xffffffff);
        let edge = px[4 * 16] & 0xff; // center 8.28 px from the arc center: partly covered
        assert!(edge > 0 && edge < 255, "{edge}");
    }

    /// The inside-span shortcut must give the same pixels as the distance math on
    /// every pixel, for fills, strokes and shadows, at odd sizes and positions.
    #[test]
    fn fast_path_matches_brute_force() {
        let (w, h) = (64, 48);
        let shapes = [
            (Rect::new(3.3, 2.7, 50.4, 40.1), 16.0),
            (Rect::new(10.0, 10.0, 30.0, 20.0), 0.0),
            (Rect::new(5.5, 4.0, 20.0, 9.0), 4.5),
        ];
        for (r, rad) in shapes {
            for kind in 0..3 {
                let mut fast = vec![0xff20_2020u32; w * h];
                let mut p = canvas(&mut fast, w, h);
                let color = 0xc0c0_c0c0; // translucent: exercises "over"
                let cov: Box<dyn Fn(f32) -> f32> = match kind {
                    0 => {
                        p.rect(r, rad, color);
                        Box::new(|d| (0.5 - d).clamp(0.0, 1.0))
                    }
                    1 => {
                        p.stroke(r, rad, 1.0, color);
                        Box::new(|d| (0.5 - d).clamp(0.0, 1.0) - (-0.5 - d).clamp(0.0, 1.0))
                    }
                    _ => {
                        p.shadow(r, rad, 14.0, 0.45);
                        Box::new(|d| {
                            let t = (d / 14.0).clamp(0.0, 1.0);
                            if d < 0.0 {
                                0.0
                            } else {
                                0.45 * (1.0 - t * t * (3.0 - 2.0 * t))
                            }
                        })
                    }
                };
                let src = if kind == 2 { 0xff00_0000 } else { color };
                let rad = rad.clamp(0.0, r.w.min(r.h) / 2.0);
                for y in 0..h {
                    for x in 0..w {
                        let d = sdf(&r, rad, x as f32 + 0.5, y as f32 + 0.5);
                        let c = (cov(d) * 255.0 + 0.5) as u32;
                        let want = if c > 0 {
                            over(0xff20_2020, src, c.min(255))
                        } else {
                            0xff20_2020
                        };
                        assert_eq!(fast[y * w + x], want, "kind {kind} {r:?} at {x},{y}");
                    }
                }
            }
        }
    }
}
