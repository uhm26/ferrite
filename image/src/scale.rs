//! Resample the visible part of an image at any zoom, write opaque ARGB8888.
//! Separable triangle filter in fixed point: bilinear when enlarging, widened to the
//! shrink factor when shrinking (so downscales don't alias).
//! Copied from ../wall/src/scale.rs at wall 3795cf6 (cover fit → scale + crop).

use crate::image::Rgb;

/// Weight precision: one output pixel's weights sum to exactly `1 << BITS`.
const BITS: u32 = 14;
/// Extra precision kept between the two passes (u16 holds 255 << 8).
const MID: u32 = 8;

/// Filter taps for one axis: output pixel `i` reads `len[i]` source pixels from
/// `start[i]`, with weights `w[i * stride ..]`.
struct Taps {
    start: Vec<usize>,
    len: Vec<usize>,
    w: Vec<i32>,
    stride: usize,
}

/// `src` source pixels → `dst` output pixels at `scale`, skipping `crop` source pixels.
fn taps(src: usize, dst: usize, scale: f64, crop: f64) -> Taps {
    let inv = 1.0 / scale;
    let radius = inv.max(1.0);
    let stride = (2.0 * radius).ceil() as usize + 1;
    let mut t = Taps {
        start: vec![0; dst],
        len: vec![0; dst],
        w: vec![0; dst * stride],
        stride,
    };
    let mut f = vec![0.0f64; stride];
    for i in 0..dst {
        let center = crop + (i as f64 + 0.5) * inv - 0.5;
        let lo = ((center - radius).floor() as isize + 1).max(0) as usize;
        let hi = ((center + radius).ceil() as isize - 1).clamp(0, src as isize - 1) as usize;
        let lo = lo.min(hi);
        let n = (hi - lo + 1).min(stride);
        let mut sum = 0.0;
        for (k, fk) in f.iter_mut().enumerate().take(n) {
            *fk = (1.0 - ((lo + k) as f64 - center).abs() / radius).max(0.0);
            sum += *fk;
        }
        let row = &mut t.w[i * stride..i * stride + n];
        normalize(&f[..n], sum, row);
        t.start[i] = lo;
        t.len[i] = n;
    }
    t
}

/// Float weights → integers summing to exactly `1 << BITS`.
fn normalize(f: &[f64], sum: f64, out: &mut [i32]) {
    let one = 1i32 << BITS;
    if sum <= 0.0 {
        out[0] = one; // degenerate: nearest pixel
        return;
    }
    for (o, &x) in out.iter_mut().zip(f) {
        *o = (x / sum * f64::from(one)).round() as i32;
    }
    let err = one - out.iter().sum::<i32>();
    if let Some(big) = out.iter_mut().max() {
        *big += err;
    }
}

/// Fill `out` (dw × dh, opaque ARGB) with `img` scaled by `scale`, starting `crop`
/// source pixels in from the top left: only the part of the image that is on screen.
/// At scale 1 with a whole-pixel crop every output pixel is one source pixel (exact).
pub fn resample(
    img: &Rgb,
    scale: f64,
    crop: (f64, f64),
    (dw, dh): (usize, usize),
    out: &mut [u32],
) {
    let tx = taps(img.w, dw, scale, crop.0);
    let ty = taps(img.h, dh, scale, crop.1);
    // Horizontal results for the last `ty.stride` source rows only: an output row
    // reads at most that many consecutive rows, and row windows only move down.
    // Each needed source row is resampled once; rows no output reads are skipped.
    let row = dw * 3;
    let mut ring = vec![0u16; ty.stride * row];
    let mut acc = vec![0i32; row];
    let mut next = 0; // first source row not resampled yet
    for (y, line) in out.chunks_exact_mut(dw).enumerate() {
        let end = ty.start[y] + ty.len[y];
        for r in next.max(ty.start[y])..end {
            let slot = r % ty.stride * row;
            horizontal(img, &tx, r, &mut ring[slot..slot + row]);
        }
        next = next.max(end);
        vertical(&ring, &ty, y, &mut acc, line);
    }
}

/// Resample source row `y` across into `line`: u16 RGB, `MID` extra bits.
fn horizontal(img: &Rgb, tx: &Taps, y: usize, line: &mut [u16]) {
    let round = 1 << (BITS - MID - 1);
    let src = &img.px[y * img.w * 3..(y + 1) * img.w * 3];
    for (x, px) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        let w = &tx.w[x * tx.stride..x * tx.stride + tx.len[x]];
        let s = &src[tx.start[x] * 3..(tx.start[x] + tx.len[x]) * 3];
        let (mut r, mut g, mut b) = (round, round, round);
        for (wk, p) in w.iter().zip(s.as_chunks::<3>().0.iter()) {
            r += i32::from(p[0]) * wk;
            g += i32::from(p[1]) * wk;
            b += i32::from(p[2]) * wk;
        }
        let shift = BITS - MID;
        px.copy_from_slice(&[
            (r >> shift) as u16,
            (g >> shift) as u16,
            (b >> shift) as u16,
        ]);
    }
}

/// Output row `y`: down the columns of the rows in `ring`, packed into `line`.
fn vertical(ring: &[u16], ty: &Taps, y: usize, acc: &mut [i32], line: &mut [u32]) {
    let row = acc.len();
    acc.fill(1 << (BITS + MID - 1));
    for k in 0..ty.len[y] {
        let wk = ty.w[y * ty.stride + k];
        let slot = (ty.start[y] + k) % ty.stride * row;
        for (a, &m) in acc.iter_mut().zip(&ring[slot..slot + row]) {
            *a += i32::from(m) * wk;
        }
    }
    for (o, c) in line.iter_mut().zip(acc.as_chunks::<3>().0.iter()) {
        let ch = |v: i32| (v >> (BITS + MID)).clamp(0, 255) as u8;
        *o = pack(ch(c[0]), ch(c[1]), ch(c[2]));
    }
}

fn pack(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, c: [u8; 3]) -> Rgb {
        Rgb {
            w,
            h,
            px: c.repeat(w * h),
        }
    }

    #[test]
    fn solid_color_survives_any_scale() {
        for (sw, sh, dw, dh) in [
            (3840, 2160, 1920, 1080),
            (800, 600, 1920, 1080),
            (1000, 3000, 64, 36),
        ] {
            let mut out = vec![0; dw * dh];
            let scale = dw as f64 / sw as f64;
            resample(
                &solid(sw, sh, [10, 128, 250]),
                scale,
                (0.0, 0.0),
                (dw, dh),
                &mut out,
            );
            assert!(
                out.iter().all(|&p| p == 0xff0a_80fa),
                "{sw}x{sh} -> {dw}x{dh}"
            );
        }
    }

    #[test]
    fn halving_averages_pairs() {
        // 16x2 alternating black/white columns → 8x1: the finest pattern there is
        // must turn mid-gray, not alias. Edge pixels see a one-sided (renormalised)
        // filter, so only the interior is checked.
        let mut img = solid(16, 2, [0, 0, 0]);
        for y in 0..2 {
            for x in (1..16).step_by(2) {
                img.px[(y * 16 + x) * 3..(y * 16 + x) * 3 + 3].copy_from_slice(&[255; 3]);
            }
        }
        let mut out = vec![0; 8];
        resample(&img, 0.5, (0.0, 0.0), (8, 1), &mut out);
        for &p in &out[1..7] {
            let g = (p >> 8) & 0xff;
            assert!((126..=129).contains(&g), "got {g}");
        }
    }

    #[test]
    fn vertical_gradient_keeps_its_rows() {
        // Gray = source row: every output row must read the right ring slots, both
        // shrinking (windows skip rows) and enlarging (windows reuse rows). The edge
        // rows see a one-sided filter, so they are skipped.
        let (sw, sh) = (256, 256);
        let mut img = solid(sw, sh, [0; 3]);
        for (y, line) in img.px.chunks_exact_mut(sw * 3).enumerate() {
            line.fill(y as u8);
        }
        for d in [37, 100, 256 + 1, 700] {
            let mut out = vec![0; d * d];
            resample(&img, d as f64 / sh as f64, (0.0, 0.0), (d, d), &mut out);
            for y in 1..d - 1 {
                let want = ((y as f64 + 0.5) * sh as f64 / d as f64 - 0.5).clamp(0.0, 255.0);
                let got = f64::from(out[y * d + d / 2] & 0xff);
                assert!(
                    (got - want).abs() <= 1.0,
                    "{d}: row {y} got {got} want {want}"
                );
            }
        }
    }

    #[test]
    fn weights_sum_to_one() {
        let t = taps(3840, 1920, 0.5, 0.0);
        for i in 0..1920 {
            let s: i32 = t.w[i * t.stride..i * t.stride + t.len[i]].iter().sum();
            assert_eq!(s, 1 << BITS);
        }
    }

    #[test]
    fn one_to_one_crop_copies_exact_pixels() {
        // Pixel value = its source x: a crop at 1:1 must start exactly at the crop.
        let mut img = solid(64, 4, [0; 3]);
        for (i, p) in img.px.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            *p = [(i % 64) as u8; 3];
        }
        let mut out = vec![0; 10 * 2];
        resample(&img, 1.0, (20.0, 1.0), (10, 2), &mut out);
        for (x, &p) in out[..10].iter().enumerate() {
            assert_eq!(p & 0xff, 20 + x as u32);
        }
    }
}
