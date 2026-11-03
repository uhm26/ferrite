//! Outline → 8-bit coverage bitmap. Exact area coverage, no hinting: every line
//! segment adds the signed area it encloses to the right of it into an accumulation
//! buffer; a running sum along each row then gives how much of each pixel is inside.
//! Quadratic curves are cut into lines first, fine enough to stay within 1/10 px.

use super::ttf::Point;

/// Largest distance a flattened curve may stray from the real one, px.
pub const TOLERANCE: f32 = 0.1;

/// A glyph's coverage: `width × rows` bytes, 0 outside … 255 inside. `left`, `top`:
/// where its top-left pixel sits relative to the pen (x right, y up from the baseline).
pub struct Bitmap {
    pub px: Vec<u8>,
    pub width: usize,
    pub rows: usize,
    pub left: i32,
    pub top: i32,
}

/// Draw `contours` (font units, y up) scaled by `scale` (x, y) and moved right by `dx` px.
pub fn rasterize(contours: &[Vec<Point>], scale: (f32, f32), dx: f32) -> Bitmap {
    let mut lines = Vec::new();
    for c in contours {
        flatten(c, scale, dx, &mut lines);
    }
    let empty = Bitmap {
        px: Vec::new(),
        width: 0,
        rows: 0,
        left: 0,
        top: 0,
    };
    let Some(b) = bounds(&lines) else {
        return empty;
    };
    let (left, right) = (b[0].floor(), b[1].ceil());
    let (bottom, top) = (b[2].floor(), b[3].ceil());
    let (w, h) = ((right - left) as usize, (top - bottom) as usize);
    if w == 0 || h == 0 {
        return empty;
    }
    // Bitmap space: x from `left`, y down from `top`. One spare cell per row end.
    let mut acc = vec![0f32; w * h + 2];
    for &(x0, y0, x1, y1) in &lines {
        line(&mut acc, w, h, (x0 - left, top - y0), (x1 - left, top - y1));
    }
    let mut sum = 0.0;
    let px = acc[..w * h]
        .iter()
        .map(|a| {
            sum += a;
            (sum.abs().min(1.0) * 255.0 + 0.5) as u8
        })
        .collect();
    Bitmap {
        px,
        width: w,
        rows: h,
        left: left as i32,
        top: top as i32,
    }
}

/// x min, x max, y min, y max over all segment ends.
fn bounds(lines: &[(f32, f32, f32, f32)]) -> Option<[f32; 4]> {
    let mut it = lines.iter().flat_map(|&(a, b, c, d)| [(a, b), (c, d)]);
    let (x, y) = it.next()?;
    Some(it.fold([x, x, y, y], |[x0, x1, y0, y1], (x, y)| {
        [x0.min(x), x1.max(x), y0.min(y), y1.max(y)]
    }))
}

/// One closed contour as line segments in px (y up). Two control points in a row
/// imply an on-curve point halfway between them (TrueType's rule).
fn flatten(c: &[Point], (sx, sy): (f32, f32), dx: f32, out: &mut Vec<(f32, f32, f32, f32)>) {
    let n = c.len();
    if n < 2 {
        return;
    }
    let px = |p: &Point| (p.x * sx + dx, p.y * sy);
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    // Start on the curve: the first on point, or the middle of the first two controls.
    let first_on = c.iter().position(|p| p.on);
    let (start, from) = match first_on {
        Some(i) => (px(&c[i]), i),
        None => (mid(px(&c[0]), px(&c[1])), 0),
    };
    let mut pen = start;
    let mut ctrl: Option<(f32, f32)> = None;
    for k in 1..=n {
        let p = &c[(from + k) % n];
        let q = px(p);
        match (p.on, ctrl) {
            (true, None) => out.push((pen.0, pen.1, q.0, q.1)),
            (true, Some(k)) => curve(pen, k, q, out),
            (false, None) => {
                ctrl = Some(q);
                continue;
            }
            (false, Some(k)) => {
                let m = mid(k, q);
                curve(pen, k, m, out);
                pen = m;
                ctrl = Some(q);
                continue;
            }
        }
        pen = q;
        ctrl = None;
    }
    // A contour that ends on a control point curves back to the start.
    if let Some(k) = ctrl {
        curve(pen, k, start, out);
    } else if first_on.is_none() {
        out.push((pen.0, pen.1, start.0, start.1));
    }
}

/// A quadratic Bézier as enough line segments: a curve cut into n parts strays at
/// most |p0 − 2p1 + p2| / (8 n²).
fn curve(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), out: &mut Vec<(f32, f32, f32, f32)>) {
    let dev = ((p0.0 - 2.0 * p1.0 + p2.0).powi(2) + (p0.1 - 2.0 * p1.1 + p2.1).powi(2)).sqrt();
    let n = ((dev / (8.0 * TOLERANCE)).sqrt().ceil() as usize).clamp(1, 64);
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let p = (
            u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
            u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
        );
        out.push((prev.0, prev.1, p.0, p.1));
        prev = p;
    }
}

/// Add the signed area right of segment a→b (bitmap px, y down) to `acc`.
/// Downward segments add, upward ones subtract, so a closed outline sums to its inside.
fn line(acc: &mut [f32], w: usize, h: usize, a: (f32, f32), b: (f32, f32)) {
    if a.1 == b.1 {
        return; // horizontal: encloses nothing
    }
    let (dir, a, b) = if a.1 < b.1 { (1.0, a, b) } else { (-1.0, b, a) };
    let dxdy = (b.0 - a.0) / (b.1 - a.1);
    let y_end = (b.1.ceil() as usize).min(h);
    for row in (a.1.max(0.0) as usize)..y_end {
        // The part of the segment inside this row.
        let (ya, yb) = (a.1.max(row as f32), b.1.min(row as f32 + 1.0));
        if yb <= ya {
            continue;
        }
        let (xa, xb) = (a.0 + (ya - a.1) * dxdy, a.0 + (yb - a.1) * dxdy);
        span(&mut acc[row * w..], w, (xa, xb), dir * (yb - ya));
    }
}

/// Spread a row piece from x `xa` to `xb` with height `d` over the cells it crosses:
/// cut at every pixel edge; each piece covers its cell right of its middle, and the
/// rest of its height carries over to every cell further right (via the next cell).
fn span(row: &mut [f32], w: usize, (xa, xb): (f32, f32), d: f32) {
    let (x0, x1) = if xa < xb { (xa, xb) } else { (xb, xa) };
    let x0 = x0.clamp(0.0, w as f32);
    let x1 = x1.clamp(0.0, w as f32);
    let len = x1 - x0;
    if len < 1e-6 {
        let c = (x0 as usize).min(w - 1);
        let f = x0 - c as f32;
        row[c] += d * (1.0 - f);
        row[c + 1] += d * f;
        return;
    }
    let mut x = x0;
    while x < x1 {
        let c = (x.floor() as usize).min(w - 1);
        let next = (c as f32 + 1.0).min(x1);
        let part = d * (next - x) / len;
        let f = (x + next) / 2.0 - c as f32;
        row[c] += part * (1.0 - f);
        row[c + 1] += part * f;
        x = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(x: f32, y: f32) -> Point {
        Point { x, y, on: true }
    }

    #[test]
    fn a_pixel_aligned_square_is_solid() {
        let sq = vec![vec![on(0.0, 0.0), on(4.0, 0.0), on(4.0, 4.0), on(0.0, 4.0)]];
        let b = rasterize(&sq, (1.0, 1.0), 0.0);
        assert_eq!((b.width, b.rows, b.left, b.top), (4, 4, 0, 4));
        assert!(b.px.iter().all(|&v| v == 255), "{:?}", b.px);
    }

    #[test]
    fn half_pixel_shift_gives_half_coverage_at_the_edges() {
        let sq = vec![vec![on(0.0, 0.0), on(2.0, 0.0), on(2.0, 1.0), on(0.0, 1.0)]];
        let b = rasterize(&sq, (1.0, 1.0), 0.5);
        assert_eq!((b.width, b.left), (3, 0));
        assert_eq!(b.px, [128, 255, 128]);
    }

    #[test]
    fn a_diagonal_halves_its_pixel() {
        let tri = vec![vec![on(0.0, 0.0), on(1.0, 0.0), on(0.0, 1.0)]];
        let b = rasterize(&tri, (1.0, 1.0), 0.0);
        assert_eq!(b.px, [128]);
    }

    #[test]
    fn a_circle_of_curves_has_its_area() {
        // Radius 10 from four quadratic arcs with control points at the square's corners:
        // each quarter is a triangle (r²/2) plus 2/3 of the control triangle (r²/3).
        let r = 10.0;
        let c = vec![vec![
            on(r, 0.0),
            Point {
                x: r,
                y: r,
                on: false,
            },
            on(0.0, r),
            Point {
                x: -r,
                y: r,
                on: false,
            },
            on(-r, 0.0),
            Point {
                x: -r,
                y: -r,
                on: false,
            },
            on(0.0, -r),
            Point {
                x: r,
                y: -r,
                on: false,
            },
        ]];
        let b = rasterize(&c, (1.0, 1.0), 0.0);
        let area: f32 = b.px.iter().map(|&v| v as f32 / 255.0).sum();
        let exact = 10.0 * r * r / 3.0;
        // Flattening cuts inside the curve by at most TOLERANCE along the ~63 px edge.
        assert!((area - exact).abs() < 63.0 * TOLERANCE, "{area} vs {exact}");
    }
}
