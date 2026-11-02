//! Pre-shrunk copies of the image (a mip chain): each level is half the one before,
//! built once at load (+1/3 memory). Zoomed out, the filter reads the level just
//! above the zoom, so its cost follows the window, not the photo.
//! Copied from ../image/src/mip.rs (uncommitted, after image 86fc166).

use crate::decode::Rgb;

pub struct Mip {
    /// Level 0 is the image itself.
    levels: Vec<Rgb>,
}

impl Mip {
    pub fn new(img: Rgb) -> Self {
        let mut levels = vec![img];
        while let Some(l) = levels.last().filter(|l| l.w > 1 && l.h > 1) {
            levels.push(half(l));
        }
        Mip { levels }
    }

    pub fn full(&self) -> &Rgb {
        &self.levels[0]
    }

    /// The level to draw `zoom` from, and how much smaller it is than level 0:
    /// the smallest one still at least as big as the result (scale stays in ½..1).
    pub fn pick(&self, zoom: f64) -> (&Rgb, f64) {
        let k = if zoom < 1.0 {
            ((1.0 / zoom).log2().floor() as usize).min(self.levels.len() - 1)
        } else {
            0
        };
        (&self.levels[k], f64::from(1u32 << k))
    }
}

/// 2×2 box average; an odd last row or column averages with itself.
fn half(s: &Rgb) -> Rgb {
    let (w, h) = (s.w.div_ceil(2), s.h.div_ceil(2));
    let mut px = vec![0u8; w * h * 3];
    let at = |x: usize, y: usize| (y * s.w + x) * 3;
    for (y, line) in px.chunks_exact_mut(w * 3).enumerate() {
        let (y0, y1) = (2 * y, (2 * y + 1).min(s.h - 1));
        for (x, p) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let (x0, x1) = (2 * x, (2 * x + 1).min(s.w - 1));
            let q = [at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)];
            for (c, o) in p.iter_mut().enumerate() {
                let sum: u16 = q.iter().map(|&i| u16::from(s.px[i + c])).sum();
                *o = ((sum + 2) / 4) as u8;
            }
        }
    }
    Rgb { w, h, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_down_to_a_line_and_picks_by_zoom() {
        let img = Rgb {
            w: 5,
            h: 3,
            px: [0, 0, 0, 255, 255, 255].repeat(8)[..45].to_vec(),
        };
        let m = Mip::new(img);
        let sizes: Vec<_> = m.levels.iter().map(|l| (l.w, l.h)).collect();
        assert_eq!(sizes, [(5, 3), (3, 2), (2, 1)]);
        assert_eq!(m.levels[1].px[..3], [128; 3]); // black + white mixed
        assert_eq!(m.pick(2.0).1, 1.0);
        assert_eq!(m.pick(0.6).1, 1.0);
        assert_eq!(m.pick(0.5).1, 2.0);
        assert_eq!(m.pick(0.3).1, 2.0);
        assert_eq!(m.pick(0.001).1, 4.0); // capped at the last level
    }
}
