//! One instance of a variable font (Adwaita Sans Medium, Bold… are one file): `fvar`
//! names the instance's axis values, `avar` bends them, and `gvar` moves outline
//! points by deltas that blend in by how close the instance is to each stored master.
//! Points a delta set leaves out are inferred from their neighbors (IUP).

use super::ttf::{Error, i16_at, u16_at, u32_at};

/// Normalized axis values (-1…1) of named instance `n` (1-based) in `fvar`, bent by
/// `avar` if the font has one.
pub fn instance(d: &[u8], fvar: usize, avar: Option<usize>, n: u16) -> Result<Vec<f32>, Error> {
    let axes_at = fvar + u16_at(d, fvar + 4)? as usize;
    let axes = u16_at(d, fvar + 8)? as usize;
    let (count, size) = (u16_at(d, fvar + 12)?, u16_at(d, fvar + 14)? as usize);
    if n == 0 || n > count {
        return Ok(vec![0.0; axes]);
    }
    let fixed = |at| u32_at(d, at).map(|v| v as i32 as f32 / 65536.0);
    let inst = axes_at + 20 * axes + size * (n as usize - 1);
    let mut out = Vec::with_capacity(axes);
    for i in 0..axes {
        let a = axes_at + 20 * i;
        let (min, def, max) = (fixed(a + 4)?, fixed(a + 8)?, fixed(a + 12)?);
        let v = fixed(inst + 4 + 4 * i)?.clamp(min, max);
        let norm = if v < def && def > min {
            (v - def) / (def - min)
        } else if v > def && max > def {
            (v - def) / (max - def)
        } else {
            0.0
        };
        out.push(norm);
    }
    if let Some(avar) = avar {
        bend(d, avar, &mut out)?;
    }
    Ok(out)
}

/// `avar`: a piecewise-linear map per axis.
fn bend(d: &[u8], avar: usize, coords: &mut [f32]) -> Result<(), Error> {
    let f2 = |at| i16_at(d, at).map(|v| f32::from(v) / 16384.0);
    let mut at = avar + 8;
    for c in coords.iter_mut() {
        let n = u16_at(d, at)? as usize;
        let map: Vec<(f32, f32)> = (0..n)
            .map(|k| Ok((f2(at + 2 + 4 * k)?, f2(at + 4 + 4 * k)?)))
            .collect::<Result<_, Error>>()?;
        at += 2 + 4 * n;
        if let Some(i) = map.windows(2).position(|w| *c >= w[0].0 && *c <= w[1].0) {
            let ((x0, y0), (x1, y1)) = (map[i], map[i + 1]);
            *c = if x1 > x0 {
                y0 + (*c - x0) / (x1 - x0) * (y1 - y0)
            } else {
                y0
            };
        }
    }
    Ok(())
}

/// A located `gvar` table and the instance to apply.
pub struct Vary {
    at: usize,
    axes: usize,
    shared: usize,
    long: bool,
    glyphs: usize,
    data: usize,
    coords: Vec<f32>,
}

const SHARED_POINTS: u16 = 0x8000;
const EMBEDDED_PEAK: u16 = 0x8000;
const INTERMEDIATE: u16 = 0x4000;
const PRIVATE_POINTS: u16 = 0x2000;

impl Vary {
    /// None when the instance is the default (no deltas to apply).
    pub fn new(d: &[u8], at: usize, coords: Vec<f32>) -> Result<Option<Self>, Error> {
        if coords.iter().all(|&c| c == 0.0) {
            return Ok(None);
        }
        Ok(Some(Vary {
            at,
            axes: u16_at(d, at + 4)? as usize,
            shared: at + u32_at(d, at + 8)? as usize,
            glyphs: u16_at(d, at + 12)? as usize,
            long: u16_at(d, at + 14)? & 1 != 0,
            data: at + u32_at(d, at + 16)? as usize,
            coords,
        }))
    }

    /// Deltas for every point of glyph `g`: `pts` are its points (plus the four
    /// phantom points at the end), `ends` the last point of each contour (for IUP;
    /// empty for a composite, whose "points" are its components).
    pub fn deltas(
        &self,
        d: &[u8],
        g: u16,
        pts: &[(f32, f32)],
        ends: &[usize],
    ) -> Result<Vec<(f32, f32)>, Error> {
        let mut out = vec![(0.0, 0.0); pts.len()];
        let Some(gvd) = self.glyph_data(d, g)? else {
            return Ok(out);
        };
        let head = u16_at(d, gvd)?;
        let mut serial = gvd + u16_at(d, gvd + 2)? as usize;
        let shared = if head & SHARED_POINTS != 0 {
            let (p, next) = points(d, serial)?;
            serial = next;
            p
        } else {
            None
        };
        let mut hdr = gvd + 4;
        for _ in 0..(head & 0x0fff) {
            let (size, index) = (u16_at(d, hdr)? as usize, u16_at(d, hdr + 2)?);
            let (scalar, next_hdr) = self.scalar(d, hdr, index)?;
            hdr = next_hdr;
            if scalar != 0.0 {
                let (own, at) = if index & PRIVATE_POINTS != 0 {
                    points(d, serial)?
                } else {
                    (shared.clone(), serial)
                };
                apply(d, (at, own.as_deref()), (pts, ends), scalar, &mut out)?;
            }
            serial += size;
        }
        Ok(out)
    }

    /// Where glyph `g`'s variation data starts; None if it has none.
    fn glyph_data(&self, d: &[u8], g: u16) -> Result<Option<usize>, Error> {
        let g = g as usize;
        if g >= self.glyphs {
            return Ok(None);
        }
        let off = |i: usize| -> Result<usize, Error> {
            Ok(if self.long {
                u32_at(d, self.at + 20 + 4 * i)? as usize
            } else {
                2 * u16_at(d, self.at + 20 + 2 * i)? as usize
            })
        };
        let (a, b) = (off(g)?, off(g + 1)?);
        Ok((b > a).then_some(self.data + a))
    }

    /// How much of one stored delta set applies here (0…1), and where the next tuple
    /// header starts.
    fn scalar(&self, d: &[u8], hdr: usize, index: u16) -> Result<(f32, usize), Error> {
        let f2 = |at| i16_at(d, at).map(|v| f32::from(v) / 16384.0);
        let mut at = hdr + 4;
        let peak_at = if index & EMBEDDED_PEAK != 0 {
            at += 2 * self.axes;
            hdr + 4
        } else {
            self.shared + 2 * self.axes * (index & 0x0fff) as usize
        };
        let region = (index & INTERMEDIATE != 0).then_some(at);
        if region.is_some() {
            at += 4 * self.axes;
        }
        let mut s = 1.0;
        for (i, &v) in self.coords.iter().enumerate() {
            let peak = f2(peak_at + 2 * i)?;
            let range = match region {
                Some(r) => Some((f2(r + 2 * i)?, f2(r + 2 * (self.axes + i))?)),
                None => None,
            };
            s *= axis_scalar(v, peak, range);
        }
        Ok((s, at))
    }
}

/// One axis's share: 1 where the instance sits at the peak, falling to 0 at the
/// region's edges (0 and the peak, or the intermediate start/end).
fn axis_scalar(v: f32, peak: f32, range: Option<(f32, f32)>) -> f32 {
    if peak == 0.0 || v == peak {
        return 1.0;
    }
    if v == 0.0 {
        return 0.0;
    }
    match range {
        Some((start, end)) if start <= peak && peak <= end && !(start < 0.0 && end > 0.0) => {
            if v < start || v > end {
                0.0
            } else if v < peak {
                (v - start) / (peak - start)
            } else {
                (end - v) / (end - peak)
            }
        }
        Some(_) => 1.0, // invalid region: the axis is ignored
        None if v < peak.min(0.0) || v > peak.max(0.0) => 0.0,
        None => v / peak,
    }
}

/// Packed point numbers: None means "every point". Returns them and where data goes on.
fn points(d: &[u8], mut at: usize) -> Result<(Option<Vec<usize>>, usize), Error> {
    let byte = |at: usize| d.get(at).copied().ok_or(Error::Truncated);
    let first = byte(at)?;
    at += 1;
    let count = if first & 0x80 != 0 {
        at += 1;
        (usize::from(first & 0x7f) << 8) | usize::from(byte(at - 1)?)
    } else {
        usize::from(first)
    };
    if count == 0 {
        return Ok((None, at));
    }
    let (mut out, mut p) = (Vec::with_capacity(count), 0usize);
    while out.len() < count {
        let c = byte(at)?;
        at += 1;
        for _ in 0..=(c & 0x7f) {
            let step = if c & 0x80 != 0 {
                at += 2;
                u16_at(d, at - 2)? as usize
            } else {
                at += 1;
                usize::from(byte(at - 1)?)
            };
            p += step;
            out.push(p);
        }
    }
    out.truncate(count);
    Ok((Some(out), at))
}

/// `n` packed deltas: runs of zeros, bytes or words.
fn packed(d: &[u8], mut at: usize, n: usize) -> Result<(Vec<f32>, usize), Error> {
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let c = *d.get(at).ok_or(Error::Truncated)?;
        at += 1;
        let run = usize::from(c & 0x3f) + 1;
        for _ in 0..run {
            let v = if c & 0x80 != 0 {
                0
            } else if c & 0x40 != 0 {
                at += 2;
                i16_at(d, at - 2)?
            } else {
                at += 1;
                i16::from(*d.get(at - 1).ok_or(Error::Truncated)? as i8)
            };
            out.push(f32::from(v));
        }
    }
    out.truncate(n);
    Ok((out, at))
}

/// Add one tuple's deltas (× `scalar`) to `out`; points it skips are inferred.
fn apply(
    d: &[u8],
    (at, which): (usize, Option<&[usize]>),
    (pts, ends): (&[(f32, f32)], &[usize]),
    scalar: f32,
    out: &mut [(f32, f32)],
) -> Result<(), Error> {
    let n = which.map_or(pts.len(), <[usize]>::len);
    let (xs, at) = packed(d, at, n)?;
    let (ys, _) = packed(d, at, n)?;
    let mut delta = vec![None; pts.len()];
    for k in 0..n {
        let i = which.map_or(k, |w| w[k]);
        if let Some(slot) = delta.get_mut(i) {
            *slot = Some((xs[k], ys[k]));
        }
    }
    if which.is_some() {
        infer(pts, ends, &mut delta);
    }
    for (o, dl) in out.iter_mut().zip(delta) {
        if let Some((x, y)) = dl {
            o.0 += x * scalar;
            o.1 += y * scalar;
        }
    }
    Ok(())
}

/// IUP: in each contour, a point without a delta takes one interpolated from the
/// nearest touched points before and after it (each axis on its own).
fn infer(pts: &[(f32, f32)], ends: &[usize], delta: &mut [Option<(f32, f32)>]) {
    let mut start = 0;
    for &end in ends {
        let idx: Vec<usize> = (start..=end.min(pts.len() - 1)).collect();
        start = end + 1;
        let touched: Vec<usize> = idx
            .iter()
            .copied()
            .filter(|&i| delta[i].is_some())
            .collect();
        if touched.is_empty() || touched.len() == idx.len() {
            continue;
        }
        for &i in &idx {
            if delta[i].is_some() {
                continue;
            }
            // Nearest touched before and after i, wrapping around the contour.
            let prev = touched
                .iter()
                .rev()
                .find(|&&t| t < i)
                .or(touched.last())
                .copied()
                .unwrap_or(i);
            let next = touched
                .iter()
                .find(|&&t| t > i)
                .or(touched.first())
                .copied()
                .unwrap_or(i);
            let (dp, dn) = (
                delta[prev].unwrap_or_default(),
                delta[next].unwrap_or_default(),
            );
            let x = interp(pts[i].0, (pts[prev].0, dp.0), (pts[next].0, dn.0));
            let y = interp(pts[i].1, (pts[prev].1, dp.1), (pts[next].1, dn.1));
            delta[i] = Some((x, y));
        }
    }
}

/// The delta for coordinate `c` between two touched points (coordinate, delta).
fn interp(c: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (lo, hi) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    if lo.0 == hi.0 {
        return if lo.1 == hi.1 { lo.1 } else { 0.0 };
    }
    if c <= lo.0 {
        lo.1
    } else if c >= hi.0 {
        hi.1
    } else {
        lo.1 + (c - lo.0) / (hi.0 - lo.0) * (hi.1 - lo.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_follow_the_spec() {
        assert_eq!(axis_scalar(0.5, 1.0, None), 0.5);
        assert_eq!(axis_scalar(-0.5, 1.0, None), 0.0);
        assert_eq!(axis_scalar(0.0, 1.0, None), 0.0);
        assert_eq!(axis_scalar(0.7, 0.0, None), 1.0);
        assert_eq!(axis_scalar(0.25, 0.5, Some((0.0, 1.0))), 0.5);
        assert_eq!(axis_scalar(0.75, 0.5, Some((0.0, 1.0))), 0.5);
        assert_eq!(axis_scalar(1.0, 0.5, Some((0.0, 0.8))), 0.0);
    }

    #[test]
    fn packed_points_and_deltas() {
        // 3 points: run of 3 bytes, steps 1, 2, 1 → points 1, 3, 4.
        let d = [3, 0x02, 1, 2, 1];
        assert_eq!(points(&d, 0).unwrap(), (Some(vec![1, 3, 4]), 5));
        assert_eq!(points(&[0], 0).unwrap(), (None, 1));
        // 2 zeros, then 1 word (-2), then 1 byte (5).
        let d = [0x81, 0x40, 0xff, 0xfe, 0x00, 5];
        assert_eq!(packed(&d, 0, 4).unwrap().0, [0.0, 0.0, -2.0, 5.0]);
    }

    #[test]
    fn untouched_points_are_interpolated() {
        let pts = [(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let mut delta = [Some((2.0, 0.0)), None, Some((4.0, 0.0)), None];
        infer(&pts, &[3], &mut delta);
        assert_eq!(delta[1], Some((3.0, 0.0))); // halfway between 2 and 4
        assert_eq!(delta[3], Some((4.0, 0.0))); // beyond both: takes the nearer end's delta
    }
}
