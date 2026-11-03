//! GIF (still or animated) through the `gif` crate, composited into whole frames
//! (the crate hands back sub-rectangles plus a disposal method per frame).

use gif::{ColorOutput, DecodeOptions, DisposalMethod};
use zune_core::colorspace::ColorSpace;

use crate::decode::{Each, Error, Frame, MAX_FRAME_BYTES, delay_ms, to_rgb};

/// Hands every frame to `each` as it is decoded.
pub fn frames(bytes: &[u8], bg: [u8; 3], each: Each) -> Result<(), Error> {
    let err = |e: gif::DecodingError| Error::Decode(format!("GIF: {e}"));
    let mut o = DecodeOptions::new();
    o.set_color_output(ColorOutput::RGBA);
    let mut d = o.read_info(bytes).map_err(err)?;
    let (w, h) = (usize::from(d.width()), usize::from(d.height()));
    if w == 0 || h == 0 {
        return Err(Error::Decode("GIF: empty canvas".into()));
    }
    let mut canvas = vec![0u8; w * h * 4];
    let mut n = 0;
    // shortcut: frames past MAX_FRAME_BYTES are dropped (the animation loops early).
    while n * w * h * 3 < MAX_FRAME_BYTES {
        let f = match d.read_next_frame() {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) if n > 0 => break, // truncated file: keep what decoded
            Err(e) => return Err(err(e)),
        };
        let rect = Rect::of(f, (w, h));
        let before = (f.dispose == DisposalMethod::Previous).then(|| canvas.clone());
        rect.draw(&mut canvas, w, &f.buffer);
        let img = to_rgb(canvas.clone(), (w, h), ColorSpace::RGBA, bg)?;
        let ms = delay_ms(u32::from(f.delay) * 10);
        n += 1;
        if !each(Frame { img, ms }) {
            return Ok(());
        }
        match (f.dispose, before) {
            (DisposalMethod::Background, _) => rect.clear(&mut canvas, w),
            (DisposalMethod::Previous, Some(b)) => canvas = b,
            _ => {}
        }
    }
    if n == 0 {
        return Err(Error::Decode("GIF: no frames".into()));
    }
    Ok(())
}

/// A frame's sub-rectangle, clipped to the canvas.
struct Rect {
    x: usize,
    y: usize,
    /// The frame's own width (row length in its buffer).
    fw: usize,
    w: usize,
    h: usize,
}

impl Rect {
    fn of(f: &gif::Frame, (cw, ch): (usize, usize)) -> Self {
        let (x, y) = (usize::from(f.left), usize::from(f.top));
        let fw = usize::from(f.width);
        Rect {
            x,
            y,
            fw,
            w: fw.min(cw.saturating_sub(x)),
            h: usize::from(f.height).min(ch.saturating_sub(y)),
        }
    }

    /// Paint the frame's opaque pixels (RGBA `buf`) over the canvas.
    fn draw(&self, canvas: &mut [u8], cw: usize, buf: &[u8]) {
        for row in 0..self.h {
            for col in 0..self.w {
                let s = (row * self.fw + col) * 4;
                let Some(p) = buf.get(s..s + 4) else { return };
                if p[3] != 0 {
                    let d = ((self.y + row) * cw + self.x + col) * 4;
                    canvas[d..d + 4].copy_from_slice(p);
                }
            }
        }
    }

    /// "Restore to background": browsers clear to transparent.
    fn clear(&self, canvas: &mut [u8], cw: usize) {
        for row in 0..self.h {
            let d = ((self.y + row) * cw + self.x) * 4;
            canvas[d..d + self.w * 4].fill(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×1 GIF, two frames: red at x 0 (dispose Background), then green at x 1.
    fn two_frames() -> Vec<u8> {
        let mut out = Vec::new();
        let pal = [255, 0, 0, 0, 255, 0];
        let mut e = gif::Encoder::new(&mut out, 2, 1, &pal).unwrap();
        for (x, i, dispose) in [(0, 0, DisposalMethod::Background), (1, 1, DisposalMethod::Keep)] {
            let mut f = gif::Frame::from_indexed_pixels(1, 1, vec![i], None);
            (f.left, f.delay, f.dispose) = (x, 5, dispose);
            e.write_frame(&f).unwrap();
        }
        drop(e);
        out
    }

    #[test]
    fn frames_are_composited_and_disposed() {
        let f = crate::decode::collect(|e| frames(&two_frames(), [9, 9, 9], e)).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].img.px, [255, 0, 0, 9, 9, 9]);
        assert_eq!(f[1].img.px, [9, 9, 9, 0, 255, 0]); // red cleared by its disposal
        assert_eq!(f[0].ms, 50);
    }

    #[test]
    fn stops_when_told() {
        let mut seen = 0;
        frames(&two_frames(), [0; 3], &mut |_| {
            seen += 1;
            false
        })
        .unwrap();
        assert_eq!(seen, 1);
    }
}
