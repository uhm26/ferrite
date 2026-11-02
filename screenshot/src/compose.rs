//! Lay the captured outputs out as one image at their compositor positions and turn
//! their pixels into RGB rows for the encoder. Gaps between outputs stay black.
//! shortcut: assumes every output has scale 1 and no transform (true here); mixed
//! scales would overlap. Upgrade by placing buffers via xdg-output logical sizes.

use crate::capture::Shot;

pub struct Canvas<'a> {
    pub w: u32,
    pub h: u32,
    x: i32,
    y: i32,
    shots: &'a [Shot],
}

impl<'a> Canvas<'a> {
    /// The bounding box of all shots. `shots` must not be empty.
    pub fn new(shots: &'a [Shot]) -> Self {
        let x = shots.iter().map(|s| s.x).min().unwrap_or(0);
        let y = shots.iter().map(|s| s.y).min().unwrap_or(0);
        let x1 = shots.iter().map(|s| s.x + s.w as i32).max().unwrap_or(0);
        let y1 = shots.iter().map(|s| s.y + s.h as i32).max().unwrap_or(0);
        Self::crop(shots, [x, y, x1 - x, y1 - y])
    }

    /// Only the layout rectangle `[x, y, w, h]` (w, h > 0).
    pub fn crop(shots: &'a [Shot], [x, y, w, h]: [i32; 4]) -> Self {
        let (w, h) = (w as u32, h as u32);
        Self { w, h, x, y, shots }
    }

    /// Write canvas row `y` as RGB into `out` (3·w bytes).
    pub fn row(&self, y: u32, out: &mut [u8]) {
        out.fill(0);
        let gy = self.y + y as i32;
        let (cx0, cx1) = (self.x, self.x + self.w as i32);
        for s in self
            .shots
            .iter()
            .filter(|s| (s.y..s.y + s.h as i32).contains(&gy))
        {
            let (x0, x1) = (cx0.max(s.x), cx1.min(s.x + s.w as i32));
            if x0 >= x1 {
                continue;
            }
            let sy = (gy - s.y) as u32;
            let sy = if s.y_invert { s.h - 1 - sy } else { sy } as usize;
            let row = &s.bytes()[sy * s.stride as usize..];
            let src = &row[(x0 - s.x) as usize * 4..(x1 - s.x) as usize * 4];
            let dst = &mut out[(x0 - cx0) as usize * 3..(x1 - cx0) as usize * 3];
            to_rgb(src, dst, s.bgr);
        }
    }
}

fn to_rgb(src: &[u8], dst: &mut [u8], bgr: bool) {
    let px = dst
        .as_chunks_mut::<3>()
        .0
        .iter_mut()
        .zip(src.as_chunks::<4>().0);
    if bgr {
        px.for_each(|(d, s)| *d = [s[2], s[1], s[0]]);
    } else {
        px.for_each(|(d, s)| *d = [s[0], s[1], s[2]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzles() {
        let mut d = [0u8; 6];
        to_rgb(&[1, 2, 3, 255, 4, 5, 6, 255], &mut d, true);
        assert_eq!(d, [3, 2, 1, 6, 5, 4]);
        to_rgb(&[1, 2, 3, 255, 4, 5, 6, 255], &mut d, false);
        assert_eq!(d, [1, 2, 3, 4, 5, 6]);
    }
}
