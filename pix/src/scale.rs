//! Resample the visible part of an image at any zoom, write opaque ARGB8888.
//! Separable triangle filter in fixed point: bilinear when enlarging, widened to the
//! shrink factor when shrinking (so downscales don't alias).
//! Copied from ../image/src/scale.rs at image 86fc166 (+ uncommitted crop/threads); `cover`
//! is wall's fit (../wall/src/scale.rs at wall 3795cf6) on top of `resample`.

use crate::decode::Rgb;

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

/// Fewest output rows worth a thread of their own.
const BAND_MIN: usize = 32;

/// Fill a dw × dh area of `out` (opaque ARGB, rows `stride` apart, starting at
/// `out[0]`) with `img` scaled by `scale`, starting `crop` source pixels in from the
/// top left: only the part of the image that is on screen. Row bands run on all
/// cores. At scale 1 with a whole-pixel crop every output pixel is one source pixel.
pub fn resample(
    img: &Rgb,
    scale: f64,
    crop: (f64, f64),
    (dw, dh): (usize, usize),
    out: (&mut [u32], usize),
) {
    let tx = taps(img.w, dw, scale, crop.0);
    run(img, &tx, &taps(img.h, dh, scale, crop.1), out);
}

/// [`resample`] without a filter: each output pixel is the source pixel under its
/// center, so an enlarged image shows crisp squares (pixel peeping).
pub fn nearest(
    img: &Rgb,
    scale: f64,
    crop: (f64, f64),
    (dw, dh): (usize, usize),
    out: (&mut [u32], usize),
) {
    let tx = sharp(img.w, dw, scale, crop.0);
    run(img, &tx, &sharp(img.h, dh, scale, crop.1), out);
}

/// One tap of full weight per output pixel: the source pixel its center falls in.
fn sharp(src: usize, dst: usize, scale: f64, crop: f64) -> Taps {
    let at = |i: usize| ((crop + (i as f64 + 0.5) / scale) as usize).min(src.saturating_sub(1));
    Taps {
        start: (0..dst).map(at).collect(),
        len: vec![1; dst],
        w: vec![1 << BITS; dst],
        stride: 1,
    }
}

/// Apply taps to the rows of `out` in bands, one per core.
fn run(img: &Rgb, tx: &Taps, ty: &Taps, (out, stride): (&mut [u32], usize)) {
    let (dw, dh) = (tx.start.len(), ty.start.len());
    if dw == 0 || dh == 0 || img.w == 0 || img.h == 0 {
        return;
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows = dh.div_ceil(cores).max(BAND_MIN);
    let out = &mut out[..(dh - 1) * stride + dw];
    std::thread::scope(|s| {
        for (b, chunk) in out.chunks_mut(rows * stride).enumerate() {
            let ys = b * rows..(b * rows + rows).min(dh);
            s.spawn(move || band(img, tx, ty, ys, (chunk, stride, dw)));
        }
    });
}

/// Fill `out` (dw × dh, packed rows) with `img` scaled to cover it, centered: the
/// overflowing sides are cut off (wallpaper fit).
pub fn cover(img: &Rgb, dw: usize, dh: usize, out: &mut [u32]) {
    let (sw, sh) = (img.w as f64, img.h as f64);
    let scale = (dw as f64 / sw).max(dh as f64 / sh);
    let crop = (
        (sw - dw as f64 / scale) / 2.0,
        (sh - dh as f64 / scale) / 2.0,
    );
    resample(img, scale, crop, (dw, dh), (out, dw));
}

/// Output rows `ys` into `out` (the band's first row at `out[0]`).
fn band(
    img: &Rgb,
    tx: &Taps,
    ty: &Taps,
    ys: std::ops::Range<usize>,
    (out, stride, dw): (&mut [u32], usize, usize),
) {
    // Horizontal results for the last `ty.stride` source rows only: an output row
    // reads at most that many consecutive rows, and row windows only move down.
    // Each needed source row is resampled once; rows no output reads are skipped.
    let row = dw * 3;
    let mut ring = vec![0u16; ty.stride * row];
    let mut acc = vec![0i32; row];
    let mut next = 0; // first source row not resampled yet
    for y in ys.clone() {
        let end = ty.start[y] + ty.len[y];
        for r in next.max(ty.start[y])..end {
            let slot = r % ty.stride * row;
            horizontal(img, tx, r, &mut ring[slot..slot + row]);
        }
        next = next.max(end);
        let at = (y - ys.start) * stride;
        vertical(&ring, ty, y, &mut acc, &mut out[at..at + dw]);
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

/// Draw `img` through an affine map, bilinear: output pixel (x, y) shows the source
/// point `(m[0]·x + m[1]·y + m[2], m[3]·x + m[4]·y + m[5])` for its center (x + ½, y + ½),
/// in source px (pixel i spans i..i+1). Output pixels whose point is off the image are
/// left as they are. For pictures turned to any angle; row bands on all cores.
/// No prefilter: pick a mip level near the scale first.
pub fn transform(img: &Rgb, m: [f64; 6], (dw, dh): (usize, usize), (out, stride): (&mut [u32], usize)) {
    if dw == 0 || dh == 0 || img.w == 0 || img.h == 0 {
        return;
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows = dh.div_ceil(cores).max(BAND_MIN);
    let out = &mut out[..(dh - 1) * stride + dw];
    std::thread::scope(|s| {
        for (b, chunk) in out.chunks_mut(rows * stride).enumerate() {
            s.spawn(move || {
                for (i, line) in chunk.chunks_mut(stride).enumerate() {
                    let y = (b * rows + i) as f64 + 0.5;
                    let start = (m[0] * 0.5 + m[1] * y + m[2], m[3] * 0.5 + m[4] * y + m[5]);
                    row(img, start, (m[0], m[3]), &mut line[..dw]);
                }
            });
        }
    });
}

/// One output row: the source point starts at `p` and moves by `d` per pixel. Fixed
/// point (16 fraction bits for the point, 8 for the blend weights) keeps it ~3 ns/px.
fn row(img: &Rgb, p: (f64, f64), d: (f64, f64), line: &mut [u32]) {
    const ONE: f64 = 65536.0;
    let (w, h) = (img.w as i64, img.h as i64);
    let (mut x, mut y) = ((p.0 * ONE) as i64, (p.1 * ONE) as i64);
    let (dx, dy) = ((d.0 * ONE) as i64, (d.1 * ONE) as i64);
    for o in line.iter_mut() {
        if x >= 0 && y >= 0 && x < w << 16 && y < h << 16 {
            // Between the four pixel centers around the point (centers sit at i + ½).
            let (fx, fy) = ((x - 32768).max(0), (y - 32768).max(0));
            let (x0, y0) = ((fx >> 16) as usize, (fy >> 16) as usize);
            let (x1, y1) = ((x0 + 1).min(img.w - 1), (y0 + 1).min(img.h - 1));
            let (tx, ty) = (((fx >> 8) & 0xff) as u32, ((fy >> 8) & 0xff) as u32);
            let (r0, r1) = (y0 * img.w, y1 * img.w);
            let at = |i: usize| &img.px[i * 3..i * 3 + 3];
            let (a, b, c, e) = (at(r0 + x0), at(r0 + x1), at(r1 + x0), at(r1 + x1));
            let ch = |k: usize| {
                let top = u32::from(a[k]) * (256 - tx) + u32::from(b[k]) * tx;
                let bottom = u32::from(c[k]) * (256 - tx) + u32::from(e[k]) * tx;
                ((top * (256 - ty) + bottom * ty + 32768) >> 16) as u8
            };
            *o = pack(ch(0), ch(1), ch(2));
        }
        x += dx;
        y += dy;
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
                (&mut out, dw),
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
        resample(&img, 0.5, (0.0, 0.0), (8, 1), (&mut out, 8));
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
            resample(
                &img,
                d as f64 / sh as f64,
                (0.0, 0.0),
                (d, d),
                (&mut out, d),
            );
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
        resample(&img, 1.0, (20.0, 1.0), (10, 2), (&mut out, 10));
        for (x, &p) in out[..10].iter().enumerate() {
            assert_eq!(p & 0xff, 20 + x as u32);
        }
    }

    #[test]
    fn cover_cuts_the_overflowing_sides_evenly() {
        // 4×2 into 2×2: scale 1, the outer columns (red) are cut, the middle stays.
        let mut img = solid(4, 2, [0, 255, 0]);
        for y in 0..2 {
            for x in [0, 3] {
                img.px[(y * 4 + x) * 3..(y * 4 + x) * 3 + 3].copy_from_slice(&[255, 0, 0]);
            }
        }
        let mut out = vec![0; 4];
        cover(&img, 2, 2, &mut out);
        assert_eq!(out, [0xff00_ff00; 4]);
    }

    #[test]
    fn nearest_repeats_pixels_without_blending() {
        // 2×1 black | white at 4× → 8×4: four black columns, then four white.
        let mut img = solid(2, 1, [0; 3]);
        img.px[3..].copy_from_slice(&[255; 3]);
        let mut out = vec![0; 8 * 4];
        nearest(&img, 4.0, (0.0, 0.0), (8, 4), (&mut out, 8));
        for row in out.chunks(8) {
            assert_eq!(row[..4], [0xff00_0000; 4]);
            assert_eq!(row[4..], [0xffff_ffff; 4]);
        }
    }

    #[test]
    fn transform_maps_turns_and_leaves_the_outside() {
        // 2×1: black | white. Identity copies; a quarter turn puts it upright; the
        // part of the output off the image keeps what was there.
        let mut img = solid(2, 1, [0; 3]);
        img.px[3..].copy_from_slice(&[255; 3]);
        let mut out = vec![7; 3];
        transform(&img, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0], (3, 1), (&mut out, 3));
        assert_eq!(out, [0xff00_0000, 0xffff_ffff, 7]);
        // Output column x=0, rows y: source (y, 0) → top black, bottom white.
        let mut out = vec![7; 2];
        transform(&img, [0.0, 1.0, 0.0, 1.0, 0.0, 0.0], (1, 2), (&mut out, 1));
        assert_eq!(out, [0xff00_0000, 0xffff_ffff]);
    }
}
