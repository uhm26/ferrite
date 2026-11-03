//! TrueType font files: the tables needed to draw text. Character → glyph (`cmap`
//! formats 4 and 12), glyph outlines (`glyf`, simple and composite, or `CFF ` via [`cff`]),
//! advances (`hmtx`)
//! and line metrics (`hhea`). Font collections (`.ttc`) by index. No hinting
//! bytecode; variations through [`vary`] (glyf fonts only).

use std::fmt;

use super::cff::Cff;
use super::vary::{self, Vary};

#[derive(Debug, PartialEq)]
pub enum Error {
    /// Shorter than its tables claim, or a table points outside the file.
    Truncated,
    /// A table this reader needs is missing.
    Missing(&'static str),
    /// No Unicode `cmap` subtable in a format we read.
    NoCmap,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Truncated => write!(f, "font file is truncated"),
            Error::Missing(t) => write!(f, "font has no {t} table"),
            Error::NoCmap => write!(f, "font has no Unicode character map"),
        }
    }
}

/// A point of an outline in font units, `on` the curve or a quadratic control point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub on: bool,
}

/// Composite glyphs nest; real fonts go 2–3 deep. Stops loops in broken files.
const MAX_DEPTH: u8 = 8;

/// Big-endian reads that fail instead of panicking on short data.
pub(super) fn u16_at(d: &[u8], at: usize) -> Result<u16, Error> {
    let b = d.get(at..at + 2).ok_or(Error::Truncated)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

pub(super) fn i16_at(d: &[u8], at: usize) -> Result<i16, Error> {
    u16_at(d, at).map(|v| v as i16)
}

pub(super) fn u32_at(d: &[u8], at: usize) -> Result<u32, Error> {
    let b = d.get(at..at + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A font file's bytes: mapped from disk (shared with every other process using it,
/// read in only where touched) or owned (tests).
pub enum Bytes {
    Mapped(crate::sys::mem::Map),
    Owned(Vec<u8>),
}

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Self {
        Bytes::Owned(v)
    }
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Mapped(m) => m.bytes(),
            Bytes::Owned(v) => v,
        }
    }
}

/// One font, its tables located in `data`.
pub struct Font {
    data: Bytes,
    glyf: (usize, usize),
    loca: (usize, usize),
    hmtx: (usize, usize),
    cmap: Cmap,
    long_loca: bool,
    /// CFF outlines instead of `glyf` (an .otf font).
    cff: Option<Cff>,
    long_metrics: u16,
    pub glyphs: u16,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    /// The named instance of a variable font, if not the default one.
    vary: Option<Vary>,
}

/// The chosen Unicode subtable: where it starts and its format (4 or 12).
struct Cmap {
    at: usize,
    format: u16,
}

impl Font {
    /// Read font `index` of a file: the low 16 bits pick a font of a collection, the
    /// high 16 a named instance of a variable font (1-based; fontconfig's numbering).
    pub fn parse(data: impl Into<Bytes>, index: u32) -> Result<Self, Error> {
        let data: Bytes = data.into();
        let base = if data.get(..4) == Some(b"ttcf") {
            u32_at(&data, 12 + 4 * (index & 0xffff) as usize)? as usize
        } else {
            0
        };
        let table = |tag: &'static str| find_table(&data, base, tag);
        let head = table("head")?.0;
        let hhea = table("hhea")?.0;
        let maxp = table("maxp")?.0;
        let hmtx = table("hmtx")?;
        let (glyf, loca, cff) = match table("glyf") {
            Ok(glyf) => (glyf, table("loca")?, None),
            Err(_) => ((0, 0), (0, 0), Some(Cff::parse(&data, table("CFF ")?.0)?)),
        };
        let cmap = pick_cmap(&data, table("cmap")?.0)?;
        let (asc, desc) = line_metrics(&data, hhea, table("OS/2").ok())?;
        let vary = match (table("fvar"), table("gvar")) {
            (Ok((fvar, _)), Ok((gvar, _))) => {
                let avar = table("avar").ok().map(|t| t.0);
                let coords = vary::instance(&data, fvar, avar, (index >> 16) as u16)?;
                Vary::new(&data, gvar, coords)?
            }
            _ => None,
        };
        Ok(Font {
            glyf,
            loca,
            hmtx,
            cmap,
            long_loca: i16_at(&data, head + 50)? != 0,
            cff,
            long_metrics: u16_at(&data, hhea + 34)?,
            glyphs: u16_at(&data, maxp + 4)?,
            units_per_em: u16_at(&data, head + 18)?.max(16),
            ascender: asc,
            descender: desc,
            vary,
            data,
        })
    }

    /// The glyph for `c`; 0 (the "missing" glyph) if the font lacks it.
    pub fn glyph(&self, c: char) -> u16 {
        let c = c as u32;
        let found = match self.cmap.format {
            4 => cmap4(&self.data, self.cmap.at, c),
            _ => cmap12(&self.data, self.cmap.at, c),
        };
        found
            .ok()
            .flatten()
            .filter(|&g| g < self.glyphs)
            .unwrap_or(0)
    }

    /// Advance width of glyph `g`, font units (moved by the instance's deltas).
    pub fn advance(&self, g: u16) -> f32 {
        let base = f32::from(self.hmtx_at(g).0);
        let Some(v) = &self.vary else { return base };
        let n = self
            .glyph_data(g)
            .ok()
            .map_or(0, |d| point_count(d).unwrap_or(0));
        let pts = vec![(0.0, 0.0); n + 4];
        match v.deltas(&self.data, g, &pts, &[]) {
            Ok(dl) => base + dl[n + 1].0 - dl[n].0,
            Err(_) => base,
        }
    }

    /// (advance, left side bearing) of glyph `g` from `hmtx`, font units.
    fn hmtx_at(&self, g: u16) -> (u16, i16) {
        let (at, len) = self.hmtx;
        let long = self.long_metrics.max(1);
        let adv = u16_at(&self.data, at + 4 * (g.min(long - 1) as usize)).unwrap_or(0);
        let lsb_at = if g < long {
            4 * g as usize + 2
        } else {
            4 * long as usize + 2 * (g - long) as usize
        };
        let lsb = if lsb_at + 2 <= len {
            i16_at(&self.data, at + lsb_at).unwrap_or(0)
        } else {
            0
        };
        (adv, lsb)
    }

    /// The four phantom points gvar moves with a glyph: origin and advance (then two
    /// vertical ones we don't use).
    fn phantom(&self, g: u16, d: &[u8]) -> [(f32, f32); 4] {
        let (adv, lsb) = self.hmtx_at(g);
        let x_min = if d.is_empty() {
            0
        } else {
            i16_at(d, 2).unwrap_or(0)
        };
        let x0 = f32::from(x_min) - f32::from(lsb);
        [
            (x0, 0.0),
            (x0 + f32::from(adv), 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
        ]
    }

    /// A simple glyph's contours moved by the instance's deltas.
    fn vary_simple(
        &self,
        g: u16,
        d: &[u8],
        mut cs: Vec<Vec<Point>>,
    ) -> Result<Vec<Vec<Point>>, Error> {
        let Some(v) = &self.vary else { return Ok(cs) };
        let mut pts: Vec<(f32, f32)> = cs.iter().flatten().map(|p| (p.x, p.y)).collect();
        let ends: Vec<usize> = cs
            .iter()
            .scan(0, |n, c| {
                *n += c.len();
                Some(*n - 1)
            })
            .collect();
        pts.extend(self.phantom(g, d));
        let dl = v.deltas(&self.data, g, &pts, &ends)?;
        for (p, (dx, dy)) in cs.iter_mut().flatten().zip(dl) {
            p.x += dx;
            p.y += dy;
        }
        Ok(cs)
    }

    /// Outline of glyph `g` as contours of points, font units, y up. Empty for a space;
    /// composite glyphs are flattened into their parts; CFF curves are cut into lines
    /// that stray at most `tol` font units.
    pub fn outline(&self, g: u16, tol: f32) -> Result<Vec<Vec<Point>>, Error> {
        if let Some(c) = &self.cff {
            return c.outline(&self.data, g, tol);
        }
        let mut out = Vec::new();
        self.add_glyph(g, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0, &mut out)?;
        Ok(out)
    }

    /// Top of glyph `g` (its bounding box's y max), font units; 0 without an outline.
    pub fn top(&self, g: u16) -> i16 {
        if self.cff.is_some() {
            let o = self.outline(g, 1.0).unwrap_or_default();
            return o.iter().flatten().map(|p| p.y as i16).max().unwrap_or(0);
        }
        self.glyph_data(g)
            .ok()
            .and_then(|d| i16_at(d, 8).ok())
            .unwrap_or(0)
    }

    /// Byte range of glyph `g` in `glyf` (empty: no outline, or a CFF font).
    fn glyph_data(&self, g: u16) -> Result<&[u8], Error> {
        if self.cff.is_some() {
            return Ok(&[]);
        }
        let (d, (loca, _)) = (&self.data, self.loca);
        let g = g as usize;
        let (a, b) = if self.long_loca {
            (
                u32_at(d, loca + 4 * g)? as usize,
                u32_at(d, loca + 4 * g + 4)? as usize,
            )
        } else {
            (
                2 * u16_at(d, loca + 2 * g)? as usize,
                2 * u16_at(d, loca + 2 * g + 2)? as usize,
            )
        };
        let (start, len) = self.glyf;
        if b <= a {
            return Ok(&[]);
        }
        if b > len {
            return Err(Error::Truncated);
        }
        Ok(&d[start + a..start + b])
    }

    /// Append glyph `g`, transformed by `m` (a b c d e f: x' = a·x + c·y + e), to `out`.
    fn add_glyph(
        &self,
        g: u16,
        m: [f32; 6],
        depth: u8,
        out: &mut Vec<Vec<Point>>,
    ) -> Result<(), Error> {
        let d = self.glyph_data(g)?;
        if d.is_empty() || depth > MAX_DEPTH {
            return Ok(());
        }
        let contours = i16_at(d, 0)?;
        if contours >= 0 {
            for c in self.vary_simple(g, d, simple(d, contours as usize)?)? {
                out.push(c.into_iter().map(|p| transform(p, m)).collect());
            }
            return Ok(());
        }
        let mut parts = components(d)?;
        if let Some(v) = &self.vary {
            // In a composite, gvar's "points" are the components' offsets.
            let mut pts: Vec<(f32, f32)> = parts.iter().map(|(_, pm, _)| (pm[4], pm[5])).collect();
            pts.extend(self.phantom(g, d));
            let dl = v.deltas(&self.data, g, &pts, &[])?;
            for ((_, pm, xy), (dx, dy)) in parts.iter_mut().zip(dl) {
                if *xy {
                    pm[4] += dx;
                    pm[5] += dy;
                }
            }
        }
        for (part, pm, _) in parts {
            self.add_glyph(part, compose(m, pm), depth + 1, out)?;
        }
        Ok(())
    }
}

/// Ascender and descender: from `hhea`, unless `OS/2` sets USE_TYPO_METRICS
/// (fsSelection bit 7), as FreeType does.
fn line_metrics(d: &[u8], hhea: usize, os2: Option<(usize, usize)>) -> Result<(i16, i16), Error> {
    if let Some((at, len)) = os2
        && len >= 72
        && u16_at(d, at + 62)? & 0x80 != 0
    {
        return Ok((i16_at(d, at + 68)?, i16_at(d, at + 70)?));
    }
    Ok((i16_at(d, hhea + 4)?, i16_at(d, hhea + 6)?))
}

/// (offset, length) of table `tag` of the font starting at `base`.
fn find_table(d: &[u8], base: usize, tag: &'static str) -> Result<(usize, usize), Error> {
    let n = u16_at(d, base + 4)? as usize;
    for i in 0..n {
        let rec = base + 12 + 16 * i;
        if d.get(rec..rec + 4) == Some(tag.as_bytes()) {
            let (at, len) = (u32_at(d, rec + 8)? as usize, u32_at(d, rec + 12)? as usize);
            if at.checked_add(len).is_none_or(|end| end > d.len()) {
                return Err(Error::Truncated);
            }
            return Ok((at, len));
        }
    }
    Err(Error::Missing(tag))
}

/// The best Unicode subtable: full range (format 12) over BMP only (format 4).
fn pick_cmap(d: &[u8], cmap: usize) -> Result<Cmap, Error> {
    let n = u16_at(d, cmap + 2)? as usize;
    let mut best: Option<Cmap> = None;
    for i in 0..n {
        let rec = cmap + 4 + 8 * i;
        let (platform, encoding) = (u16_at(d, rec)?, u16_at(d, rec + 2)?);
        let unicode = platform == 0 || (platform == 3 && (encoding == 1 || encoding == 10));
        let at = cmap + u32_at(d, rec + 4)? as usize;
        let format = u16_at(d, at)?;
        if unicode && (format == 12 || (format == 4 && best.is_none())) {
            best = Some(Cmap { at, format });
        }
    }
    best.ok_or(Error::NoCmap)
}

/// Format 4: segments of BMP characters, by delta or through a glyph array.
fn cmap4(d: &[u8], at: usize, c: u32) -> Result<Option<u16>, Error> {
    if c > 0xffff {
        return Ok(None);
    }
    let segs = u16_at(d, at + 6)? as usize / 2;
    let (ends, starts) = (at + 14, at + 16 + 2 * segs);
    let (deltas, ranges) = (starts + 2 * segs, starts + 4 * segs);
    // Ends are sorted: binary search for the first segment ending at or after c.
    let (mut lo, mut hi) = (0, segs);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if u32::from(u16_at(d, ends + 2 * mid)?) < c {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    if lo == segs || u32::from(u16_at(d, starts + 2 * lo)?) > c {
        return Ok(None);
    }
    let delta = u16_at(d, deltas + 2 * lo)?;
    let range = u16_at(d, ranges + 2 * lo)? as usize;
    if range == 0 {
        return Ok(Some((c as u16).wrapping_add(delta)));
    }
    let start = u32::from(u16_at(d, starts + 2 * lo)?);
    let g = u16_at(d, ranges + 2 * lo + range + 2 * (c - start) as usize)?;
    Ok((g != 0).then(|| g.wrapping_add(delta)))
}

/// Format 12: sorted groups of characters mapped to consecutive glyphs.
fn cmap12(d: &[u8], at: usize, c: u32) -> Result<Option<u16>, Error> {
    let n = u32_at(d, at + 12)? as usize;
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let g = at + 16 + 12 * mid;
        let (first, last) = (u32_at(d, g)?, u32_at(d, g + 4)?);
        if c < first {
            hi = mid;
        } else if c > last {
            lo = mid + 1;
        } else {
            return Ok(Some((u32_at(d, g + 8)? + c - first) as u16));
        }
    }
    Ok(None)
}

const ON_CURVE: u8 = 1;
const X_SHORT: u8 = 2;
const Y_SHORT: u8 = 4;
const REPEAT: u8 = 8;
const X_SAME: u8 = 16;
const Y_SAME: u8 = 32;

/// A simple glyph's contours: flags (run-length), then x deltas, then y deltas.
fn simple(d: &[u8], contours: usize) -> Result<Vec<Vec<Point>>, Error> {
    let mut ends = Vec::with_capacity(contours);
    for i in 0..contours {
        ends.push(u16_at(d, 10 + 2 * i)? as usize);
    }
    let points = ends.last().map_or(0, |e| e + 1);
    let code = 10 + 2 * contours;
    let mut at = code + 2 + u16_at(d, code)? as usize;
    let mut flags = Vec::with_capacity(points);
    while flags.len() < points {
        let f = *d.get(at).ok_or(Error::Truncated)?;
        let n = if f & REPEAT != 0 {
            at += 1;
            1 + *d.get(at).ok_or(Error::Truncated)? as usize
        } else {
            1
        };
        flags.extend(std::iter::repeat_n(f, n));
        at += 1;
    }
    flags.truncate(points);
    let (xs, at) = coords(d, at, &flags, X_SHORT, X_SAME)?;
    let (ys, _) = coords(d, at, &flags, Y_SHORT, Y_SAME)?;
    let mut out = Vec::with_capacity(contours);
    let mut start = 0;
    for end in ends {
        let end = end.min(points.saturating_sub(1));
        out.push(
            (start..=end)
                .map(|i| Point {
                    x: xs[i],
                    y: ys[i],
                    on: flags[i] & ON_CURVE != 0,
                })
                .collect(),
        );
        start = end + 1;
    }
    Ok(out)
}

/// One axis of coordinates: each a byte (sign in `same`), a 16-bit delta, or unchanged.
fn coords(
    d: &[u8],
    mut at: usize,
    flags: &[u8],
    short: u8,
    same: u8,
) -> Result<(Vec<f32>, usize), Error> {
    let mut v = 0i32;
    let mut out = Vec::with_capacity(flags.len());
    for &f in flags {
        if f & short != 0 {
            let b = i32::from(*d.get(at).ok_or(Error::Truncated)?);
            v += if f & same != 0 { b } else { -b };
            at += 1;
        } else if f & same == 0 {
            v += i32::from(i16_at(d, at)?);
            at += 2;
        }
        out.push(v as f32);
    }
    Ok((out, at))
}

const WORDS: u16 = 1;
const XY_VALUES: u16 = 2;
const SCALE: u16 = 8;
const MORE_COMPONENTS: u16 = 0x20;
const XY_SCALE: u16 = 0x40;
const TWO_BY_TWO: u16 = 0x80;

/// A composite glyph's parts: (glyph, transform, placed by x/y offset).
fn components(d: &[u8]) -> Result<Vec<(u16, [f32; 6], bool)>, Error> {
    let (mut at, mut out) = (10, Vec::new());
    loop {
        let (flags, part) = (u16_at(d, at)?, u16_at(d, at + 2)?);
        let (pm, next) = component(d, at + 4, flags)?;
        out.push((part, pm, flags & XY_VALUES != 0));
        if flags & MORE_COMPONENTS == 0 || out.len() > 255 {
            return Ok(out);
        }
        at = next;
    }
}

/// How many points gvar counts for a glyph: outline points, or components.
fn point_count(d: &[u8]) -> Result<usize, Error> {
    if d.is_empty() {
        return Ok(0);
    }
    let contours = i16_at(d, 0)?;
    if contours < 0 {
        return Ok(components(d)?.len());
    }
    Ok(if contours == 0 { 0 } else { u16_at(d, 10 + 2 * (contours as usize - 1))? as usize + 1 })
}

/// A component's transform (a b c d e f) and where the next component starts.
/// Point-matching placement (no XY_VALUES) is rare and placed at 0, 0.
fn component(d: &[u8], at: usize, flags: u16) -> Result<([f32; 6], usize), Error> {
    let (mut e, mut f, mut at) = if flags & WORDS != 0 {
        (
            f32::from(i16_at(d, at)?),
            f32::from(i16_at(d, at + 2)?),
            at + 4,
        )
    } else {
        let b = d.get(at..at + 2).ok_or(Error::Truncated)?;
        (f32::from(b[0] as i8), f32::from(b[1] as i8), at + 2)
    };
    if flags & XY_VALUES == 0 {
        (e, f) = (0.0, 0.0);
    }
    let f2 = |at| i16_at(d, at).map(|v| f32::from(v) / 16384.0);
    let (a, b, c, dd) = if flags & SCALE != 0 {
        let s = f2(at)?;
        at += 2;
        (s, 0.0, 0.0, s)
    } else if flags & XY_SCALE != 0 {
        let (sx, sy) = (f2(at)?, f2(at + 2)?);
        at += 4;
        (sx, 0.0, 0.0, sy)
    } else if flags & TWO_BY_TWO != 0 {
        let m = (f2(at)?, f2(at + 2)?, f2(at + 4)?, f2(at + 6)?);
        at += 8;
        m
    } else {
        (1.0, 0.0, 0.0, 1.0)
    };
    Ok(([a, b, c, dd, e, f], at))
}

fn transform(p: Point, m: [f32; 6]) -> Point {
    Point {
        x: m[0] * p.x + m[2] * p.y + m[4],
        y: m[1] * p.x + m[3] * p.y + m[5],
        on: p.on,
    }
}

/// `outer` applied after `inner`.
fn compose(outer: [f32; 6], inner: [f32; 6]) -> [f32; 6] {
    let [a, b, c, d, e, f] = outer;
    let [ia, ib, ic, id, ie, iff] = inner;
    [
        a * ia + c * ib,
        b * ia + d * ib,
        a * ic + c * id,
        b * ic + d * id,
        a * ie + c * iff + e,
        b * ie + d * iff + f,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be16(v: u16) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    /// Glyph 1: square (0,0) (100,0) (100,100) (0,100). Glyph 2: glyph 1 moved right
    /// by 100. "Same" flags without "short" mean "no delta stored"; the rest are 16 bits.
    fn glyf_and_loca() -> (Vec<u8>, Vec<u8>) {
        let flags = [ON_CURVE | X_SAME | Y_SAME, ON_CURVE | Y_SAME, ON_CURVE | X_SAME, ON_CURVE | Y_SAME];
        let square = [
            be16(1),                                     // contours
            [0u16, 0, 100, 100].map(be16).concat(),      // bbox
            be16(3),                                     // last point of the contour
            be16(0),                                     // no instructions
            flags.to_vec(),
            [100, (-100i16) as u16, 100].map(be16).concat(), // x of points 1, 3; y of point 2
        ]
        .concat();
        let composite = [
            [(-1i16) as u16, 0, 0, 200, 100].map(be16).concat(),
            [WORDS | XY_VALUES, 1, 100, 0].map(be16).concat(),
        ]
        .concat();
        let glyf = [square.clone(), composite].concat();
        let loca = [0, 0, (square.len() / 2) as u16, (glyf.len() / 2) as u16].map(be16).concat();
        (glyf, loca)
    }

    /// One format 4 subtable: 'A'..'B' → glyphs 1..2, plus the closing 0xffff segment.
    fn cmap() -> Vec<u8> {
        let head = [[0u16, 1, 3, 1].map(be16).concat(), 12u32.to_be_bytes().to_vec()].concat();
        let sub = [4, 32, 0, 4, 0, 0, 0, 0x42, 0xffff, 0, 0x41, 0xffff, (1i16 - 0x41) as u16, 1, 0, 0];
        [head, sub.map(be16).concat()].concat()
    }

    /// Tables in a font file: directory, then the bodies in order.
    fn assemble(tables: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = [vec![0, 1, 0, 0], be16(tables.len() as u16), vec![0; 6]].concat();
        let mut at = 12 + 16 * tables.len();
        let mut body: Vec<u8> = Vec::new();
        for (tag, t) in tables {
            out.extend(tag.as_bytes());
            out.extend([0; 4]);
            out.extend((at as u32).to_be_bytes());
            out.extend((t.len() as u32).to_be_bytes());
            body.extend(t);
            at += t.len();
        }
        out.extend(body);
        out
    }

    /// A tiny hand-made font: 'A' is a square (glyph 1), 'B' a composite of it (glyph 2).
    pub(crate) fn tiny() -> Vec<u8> {
        let (glyf, loca) = glyf_and_loca();
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&be16(1000));
        let mut hhea = vec![0u8; 36];
        hhea[4..8].copy_from_slice(&[be16(800), be16((-200i16) as u16)].concat());
        hhea[34..36].copy_from_slice(&be16(3));
        let maxp = [0u16, 0, 3].map(be16).concat();
        let hmtx = [500u16, 0, 600, 0, 700, 0].map(be16).concat();
        assemble(&[("cmap", cmap()), ("glyf", glyf), ("head", head), ("hhea", hhea), ("hmtx", hmtx), ("loca", loca), ("maxp", maxp)])
    }

    #[test]
    fn reads_cmap_metrics_and_outlines() {
        let f = Font::parse(tiny(), 0).unwrap();
        assert_eq!((f.glyph('A'), f.glyph('B'), f.glyph('C')), (1, 2, 0));
        assert_eq!((f.units_per_em, f.ascender, f.descender), (1000, 800, -200));
        assert_eq!((f.advance(1), f.advance(2)), (600.0, 700.0));
        let sq = f.outline(1, 1.0).unwrap();
        let xy: Vec<_> = sq[0].iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(xy, [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let moved = f.outline(2, 1.0).unwrap();
        assert_eq!(moved[0][1].x, 200.0);
    }

    #[test]
    fn junk_and_short_files_fail_cleanly() {
        assert!(Font::parse(vec![0; 3], 0).is_err());
        let mut t = tiny();
        t.truncate(t.len() - 10);
        assert_eq!(Font::parse(t, 0).err(), Some(Error::Truncated));
    }
}
