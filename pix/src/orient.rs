//! Turn and flip: the EXIF orientation cameras write instead of turning the pixels,
//! and a viewer's own rotate ([`CW`], [`CCW`]).

use crate::decode::Rgb;

/// EXIF orientation that turns the picture a quarter clockwise.
pub const CW: u8 = 6;
/// A quarter counterclockwise.
pub const CCW: u8 = 8;

/// The Orientation tag (0x0112) of IFD0 in a TIFF block (`II*\0` or `MM\0*` first,
/// what JPEG's APP1 "Exif" holds): 1–8, 1 = as stored. Anything unreadable is 1.
pub(crate) fn exif_orientation(tiff: &[u8]) -> u8 {
    let le = match tiff.get(..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return 1,
    };
    let num = |i: usize, n: usize| {
        let b = tiff.get(i..i + n)?;
        let fold = |acc: u32, &x: &u8| acc << 8 | u32::from(x);
        Some(if le { b.iter().rev().fold(0, fold) } else { b.iter().fold(0, fold) })
    };
    let Some(ifd) = num(4, 4).map(|v| v as usize) else {
        return 1;
    };
    let entries = num(ifd, 2).unwrap_or(0) as usize;
    (0..entries)
        .map(|k| ifd + 2 + 12 * k)
        .find(|&e| num(e, 2) == Some(0x0112))
        // A SHORT with count 1: the value is the first 2 bytes of the value field.
        .and_then(|e| num(e + 8, 2))
        .filter(|o| (1..=8).contains(o))
        .map_or(1, |o| o as u8)
}

/// `img` turned and flipped so EXIF orientation `o` (1–8) shows upright; 6 and 8
/// swap width and height. Other values copy it unchanged.
pub fn orient(img: &Rgb, o: u8) -> Rgb {
    let (w, h) = (img.w, img.h);
    let (dw, dh) = if (5..=8).contains(&o) { (h, w) } else { (w, h) };
    // Shown pixel (x, y) ← stored pixel: the EXIF spec's eight cases.
    let from: fn(usize, usize, usize, usize) -> (usize, usize) = match o {
        2 => |x, y, w, _| (w - 1 - x, y),
        3 => |x, y, w, h| (w - 1 - x, h - 1 - y),
        4 => |x, y, _, h| (x, h - 1 - y),
        5 => |x, y, _, _| (y, x),
        6 => |x, y, _, h| (y, h - 1 - x),
        7 => |x, y, w, h| (w - 1 - y, h - 1 - x),
        8 => |x, y, w, _| (w - 1 - y, x),
        _ => |x, y, _, _| (x, y),
    };
    let mut px = vec![0; dw * dh * 3];
    if dw > 0 {
        for (y, line) in px.chunks_exact_mut(dw * 3).enumerate() {
            for (x, p) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let (sx, sy) = from(x, y, w, h);
                let i = (sy * w + sx) * 3;
                p.copy_from_slice(&img.px[i..i + 3]);
            }
        }
    }
    Rgb { w: dw, h: dh, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2×1: pixel values 1 and 2 (gray).
    fn pair() -> Rgb {
        Rgb { w: 2, h: 1, px: vec![1, 1, 1, 2, 2, 2] }
    }

    fn gray(img: &Rgb) -> Vec<u8> {
        img.px.iter().step_by(3).copied().collect()
    }

    #[test]
    fn turns_and_flips() {
        let cw = orient(&pair(), CW);
        assert_eq!((cw.w, cw.h, gray(&cw)), (1, 2, vec![1, 2])); // left → top
        let ccw = orient(&pair(), CCW);
        assert_eq!((ccw.w, ccw.h, gray(&ccw)), (1, 2, vec![2, 1]));
        assert_eq!(gray(&orient(&pair(), 2)), vec![2, 1]);
        assert_eq!(gray(&orient(&pair(), 1)), vec![1, 2]);
        let four = (0..4).fold(pair(), |i, _| orient(&i, CW));
        assert_eq!((four.w, gray(&four)), (2, vec![1, 2]));
    }

    #[test]
    fn reads_the_orientation_tag() {
        // Big endian, IFD0 at 8 with 2 entries: some tag, then Orientation = 6.
        let mut t = b"MM\0*\0\0\0\x08\0\x02".to_vec();
        t.extend_from_slice(&[0x01, 0x0f, 0, 2, 0, 0, 0, 4, b'a', b'b', b'c', 0]);
        t.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        assert_eq!(exif_orientation(&t), 6);
        let le = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x08\0\0\0";
        assert_eq!(exif_orientation(le), 8);
        assert_eq!(exif_orientation(&t[..20]), 1); // cut short
        assert_eq!(exif_orientation(b"junk"), 1);
    }
}
