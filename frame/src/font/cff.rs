//! CFF outlines (the `CFF ` table of .otf fonts, e.g. Noto CJK): each glyph is a
//! Type 2 charstring, a small stack program that draws cubic Béziers. They are
//! flattened to straight lines here (within a tolerance the caller gives), so the
//! rasterizer only ever sees polygons. CID-keyed fonts pick their subroutines per glyph
//! through FDSelect. Hints are skipped; CFF2 (variable .otf) is not read.

use super::ttf::{Error, Point, i16_at, u16_at, u32_at};

/// Subroutines call subroutines; the spec allows 10 levels.
const MAX_CALLS: u8 = 10;
/// The spec's argument stack limit.
const MAX_STACK: usize = 48;

/// An INDEX: `count` items, item i at `data + offset[i]..data + offset[i+1]`.
#[derive(Clone, Copy, Default)]
struct Index {
    count: usize,
    off_size: usize,
    offsets: usize,
    data: usize,
}

impl Index {
    /// The INDEX at `at`, and where the bytes after it start.
    fn at(d: &[u8], at: usize) -> Result<(Index, usize), Error> {
        let count = u16_at(d, at)? as usize;
        if count == 0 {
            return Ok((Index::default(), at + 2));
        }
        let off_size = usize::from(*d.get(at + 2).ok_or(Error::Truncated)?);
        if !(1..=4).contains(&off_size) {
            return Err(Error::Truncated);
        }
        let idx = Index {
            count,
            off_size,
            offsets: at + 3,
            data: at + 2 + (count + 1) * off_size,
        };
        let end = idx.data + idx.offset(d, count)?;
        Ok((idx, end))
    }

    /// Offsets are 1-based, so `data` points one byte before the first item.
    fn offset(&self, d: &[u8], i: usize) -> Result<usize, Error> {
        let at = self.offsets + i * self.off_size;
        let b = d.get(at..at + self.off_size).ok_or(Error::Truncated)?;
        Ok(b.iter().fold(0, |v, &x| (v << 8) | usize::from(x)))
    }

    fn get<'a>(&self, d: &'a [u8], i: usize) -> Result<&'a [u8], Error> {
        if i >= self.count {
            return Err(Error::Truncated);
        }
        let (a, b) = (self.offset(d, i)?, self.offset(d, i + 1)?);
        d.get(self.data + a..self.data + b).ok_or(Error::Truncated)
    }
}

/// DICT operators we read.
const CHARSTRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const SUBRS: u16 = 19;
const FD_ARRAY: u16 = 0x0c24;
const FD_SELECT: u16 = 0x0c25;

/// A DICT: operands then an operator, repeated. Real numbers are skipped (read as 0);
/// none of the operators we need take them.
fn dict(d: &[u8]) -> Result<Vec<(u16, Vec<i32>)>, Error> {
    let (mut out, mut args, mut i) = (Vec::new(), Vec::new(), 0);
    let byte = |i: usize| d.get(i).copied().ok_or(Error::Truncated).map(i32::from);
    while i < d.len() {
        let b = byte(i)?;
        i += 1;
        match b {
            0..=21 => {
                let op = if b == 12 {
                    i += 1;
                    0x0c00 | byte(i - 1)? as u16
                } else {
                    b as u16
                };
                out.push((op, std::mem::take(&mut args)));
            }
            28 => {
                args.push(i32::from(i16_at(d, i)?));
                i += 2;
            }
            29 => {
                args.push(u32_at(d, i)? as i32);
                i += 4;
            }
            30 => {
                while byte(i)? & 0x0f != 0x0f && byte(i)? >> 4 != 0x0f {
                    i += 1;
                }
                i += 1;
                args.push(0);
            }
            32..=246 => args.push(b - 139),
            247..=250 => {
                args.push((b - 247) * 256 + byte(i)? + 108);
                i += 1;
            }
            251..=254 => {
                args.push(-(b - 251) * 256 - byte(i)? - 108);
                i += 1;
            }
            _ => {} // reserved
        }
    }
    Ok(out)
}

fn find(dict: &[(u16, Vec<i32>)], op: u16) -> Option<&[i32]> {
    dict.iter()
        .find(|(o, _)| *o == op)
        .map(|(_, a)| a.as_slice())
}

/// A located CFF table: charstrings, global and local subroutines.
pub struct Cff {
    charstrings: Index,
    globals: Index,
    /// Local subroutines per font dict (one for a plain font).
    locals: Vec<Index>,
    /// FDSelect: which font dict a glyph uses (CID-keyed fonts only).
    fd_select: Option<usize>,
}

impl Cff {
    /// The CFF table at `at` (font 0 of its fontset).
    pub fn parse(d: &[u8], at: usize) -> Result<Cff, Error> {
        let header = usize::from(*d.get(at + 2).ok_or(Error::Truncated)?);
        let (_names, next) = Index::at(d, at + header)?;
        let (tops, next) = Index::at(d, next)?;
        let (_strings, next) = Index::at(d, next)?;
        let (globals, _) = Index::at(d, next)?;
        let top = dict(tops.get(d, 0)?)?;
        let off = |v: i32| at + v.max(0) as usize;
        let cs = find(&top, CHARSTRINGS)
            .and_then(|a| a.first().copied())
            .ok_or(Error::Missing("CharStrings"))?;
        let (charstrings, _) = Index::at(d, off(cs))?;
        let (locals, fd_select) = match (find(&top, FD_ARRAY), find(&top, FD_SELECT)) {
            (Some([fda]), Some([fds])) => {
                let (fdarray, _) = Index::at(d, off(*fda))?;
                let locals = (0..fdarray.count)
                    .map(|i| local_subrs(d, at, &dict(fdarray.get(d, i)?)?))
                    .collect::<Result<_, _>>()?;
                (locals, Some(off(*fds)))
            }
            _ => (vec![local_subrs(d, at, &top)?], None),
        };
        Ok(Cff {
            charstrings,
            globals,
            locals,
            fd_select,
        })
    }

    /// Glyph `g` as closed polygons (font units, y up), curves cut into lines that
    /// stray at most `tol` font units.
    pub fn outline(&self, d: &[u8], g: u16, tol: f32) -> Result<Vec<Vec<Point>>, Error> {
        let code = self.charstrings.get(d, g as usize)?;
        let locals = self.locals.get(self.fd(d, g)?).copied().unwrap_or_default();
        let mut run = Run {
            d,
            globals: self.globals,
            locals,
            tol,
            stack: Vec::new(),
            pen: (0.0, 0.0),
            stems: 0,
            width_done: false,
            out: Vec::new(),
            cur: Vec::new(),
        };
        run.exec(code, 0)?;
        run.close();
        Ok(run.out)
    }

    /// Which font dict glyph `g` uses.
    fn fd(&self, d: &[u8], g: u16) -> Result<usize, Error> {
        let Some(at) = self.fd_select else {
            return Ok(0);
        };
        match d.get(at) {
            Some(0) => Ok(usize::from(
                *d.get(at + 1 + g as usize).ok_or(Error::Truncated)?,
            )),
            Some(3) => {
                let n = u16_at(d, at + 1)? as usize;
                for r in 0..n {
                    let first = u16_at(d, at + 3 + 3 * r)?;
                    let next = u16_at(d, at + 6 + 3 * r)?;
                    if (first..next).contains(&g) {
                        return Ok(usize::from(*d.get(at + 5 + 3 * r).ok_or(Error::Truncated)?));
                    }
                }
                Ok(0)
            }
            _ => Ok(0),
        }
    }
}

/// The local subroutines of a font dict: its Private DICT's Subrs (relative to it).
fn local_subrs(d: &[u8], cff: usize, font: &[(u16, Vec<i32>)]) -> Result<Index, Error> {
    let Some([size, off]) = find(font, PRIVATE) else {
        return Ok(Index::default());
    };
    let start = cff + (*off).max(0) as usize;
    let private = d
        .get(start..start + (*size).max(0) as usize)
        .ok_or(Error::Truncated)?;
    match find(&dict(private)?, SUBRS) {
        Some([s]) => Index::at(d, start + (*s).max(0) as usize).map(|(i, _)| i),
        _ => Ok(Index::default()),
    }
}

/// Subroutine numbers are stored minus a bias that depends on how many there are.
fn bias(count: usize) -> i32 {
    match count {
        0..1240 => 107,
        1240..33900 => 1131,
        _ => 32768,
    }
}

/// One charstring being run: the stack, the pen, and the polygons drawn so far.
struct Run<'a> {
    d: &'a [u8],
    globals: Index,
    locals: Index,
    tol: f32,
    stack: Vec<f32>,
    pen: (f32, f32),
    stems: usize,
    /// The first stack-clearing operator may carry the glyph width first; skip it once.
    width_done: bool,
    out: Vec<Vec<Point>>,
    cur: Vec<Point>,
}

impl Run<'_> {
    /// Run `code`; true once `endchar` is seen.
    fn exec(&mut self, code: &[u8], depth: u8) -> Result<bool, Error> {
        if depth > MAX_CALLS {
            return Err(Error::Truncated);
        }
        let mut i = 0;
        while i < code.len() {
            let b = code[i];
            i += 1;
            if b >= 32 || b == 28 || b == 255 {
                i = self.number(code, i - 1)?;
                continue;
            }
            let op = if b == 12 {
                i += 1;
                0x0c00 | u16::from(*code.get(i - 1).ok_or(Error::Truncated)?)
            } else {
                u16::from(b)
            };
            match op {
                10 | 29 => {
                    let n = self.stack.pop().ok_or(Error::Truncated)? as i32;
                    let subrs = if op == 10 { self.locals } else { self.globals };
                    let sub = subrs.get(self.d, (n + bias(subrs.count)).max(0) as usize)?;
                    if self.exec(sub, depth + 1)? {
                        return Ok(true);
                    }
                }
                11 => return Ok(false), // return
                14 => {
                    self.width(0);
                    return Ok(true);
                }
                19 | 20 => {
                    self.stem_hints();
                    i += self.stems.div_ceil(8); // the mask bytes
                }
                _ => self.draw(op)?,
            }
        }
        Ok(false)
    }

    /// Push the number encoded at `code[i]`; returns where the next token starts.
    fn number(&mut self, code: &[u8], i: usize) -> Result<usize, Error> {
        let byte = |k: usize| code.get(k).copied().map(i32::from).ok_or(Error::Truncated);
        let b = byte(i)?;
        let (v, len) = match b {
            28 => (f32::from(i16_at(code, i + 1)?), 3),
            255 => (u32_at(code, i + 1)? as i32 as f32 / 65536.0, 5),
            32..=246 => ((b - 139) as f32, 1),
            247..=250 => (((b - 247) * 256 + byte(i + 1)? + 108) as f32, 2),
            _ => ((-(b - 251) * 256 - byte(i + 1)? - 108) as f32, 2),
        };
        if self.stack.len() < MAX_STACK {
            self.stack.push(v);
        }
        Ok(i + len)
    }

    /// Drop the leading width argument if this operator has one more than it takes.
    fn width(&mut self, takes: usize) {
        if !self.width_done {
            self.width_done = true;
            if self.stack.len() > takes && (self.stack.len() - takes) % 2 == 1 {
                self.stack.remove(0);
            }
        }
    }

    /// hstem, vstem (and the implicit vstem before a hintmask): just count stems.
    fn stem_hints(&mut self) {
        self.width(0);
        self.stems += self.stack.len() / 2;
        self.stack.clear();
    }

    fn draw(&mut self, op: u16) -> Result<(), Error> {
        let s = std::mem::take(&mut self.stack);
        match op {
            1 | 3 | 18 | 23 => {
                self.stack = s;
                self.stem_hints();
            }
            21 | 22 | 4 => self.move_op(op, s),
            5 => s.as_chunks::<2>().0.iter().for_each(|&[x, y]| self.line(x, y)),
            6 | 7 => {
                for (k, &v) in s.iter().enumerate() {
                    let horizontal = (k % 2 == 0) == (op == 6);
                    if horizontal {
                        self.line(v, 0.0)
                    } else {
                        self.line(0.0, v)
                    }
                }
            }
            8 => s.as_chunks::<6>().0.iter().for_each(|&c| self.curve(c)),
            24 | 25 => self.mixed(op, &s),
            26 | 27 => self.same_dir(op == 27, &s),
            30 | 31 => self.alternating(op == 31, &s),
            0x0c22..=0x0c25 => self.flex(op, &s),
            _ => {} // arithmetic and other rare operators: ignored
        }
        Ok(())
    }

    fn move_op(&mut self, op: u16, s: Vec<f32>) {
        self.stack = s;
        self.width(if op == 21 { 2 } else { 1 });
        let s = std::mem::take(&mut self.stack);
        let (dx, dy) = match op {
            21 => (
                s.first().copied().unwrap_or(0.0),
                s.get(1).copied().unwrap_or(0.0),
            ),
            22 => (s.first().copied().unwrap_or(0.0), 0.0),
            _ => (0.0, s.first().copied().unwrap_or(0.0)),
        };
        self.close();
        self.pen = (self.pen.0 + dx, self.pen.1 + dy);
        self.cur.push(Point {
            x: self.pen.0,
            y: self.pen.1,
            on: true,
        });
    }

    /// rcurveline (curves, then a line) and rlinecurve (lines, then a curve).
    fn mixed(&mut self, op: u16, s: &[f32]) {
        if op == 24 {
            let curves = s.len().saturating_sub(2) / 6;
            s[..6 * curves].as_chunks::<6>().0.iter().for_each(|&c| self.curve(c));
            if let [x, y] = s[6 * curves..] {
                self.line(x, y)
            }
        } else {
            let lines = s.len().saturating_sub(6) / 2;
            s[..2 * lines].as_chunks::<2>().0.iter().for_each(|&[x, y]| self.line(x, y));
            if let [a, b, c, d, e, f] = s[2 * lines..] {
                self.curve([a, b, c, d, e, f])
            }
        }
    }

    /// hhcurveto / vvcurveto: curves that start and end in one direction; an odd
    /// leading value nudges the first curve sideways.
    fn same_dir(&mut self, horizontal: bool, s: &[f32]) {
        let (mut side, mut i) = if s.len() % 4 == 1 {
            (s[0], 1)
        } else {
            (0.0, 0)
        };
        while i + 4 <= s.len() {
            let (a, b, c, e) = (s[i], s[i + 1], s[i + 2], s[i + 3]);
            if horizontal {
                self.curve([a, side, b, c, e, 0.0])
            } else {
                self.curve([side, a, b, c, 0.0, e])
            }
            side = 0.0;
            i += 4;
        }
    }

    /// hvcurveto / vhcurveto: curves turning between horizontal and vertical; the very
    /// last one may take a fifth value for its free end.
    fn alternating(&mut self, mut horizontal: bool, s: &[f32]) {
        let mut i = 0;
        while i + 4 <= s.len() {
            let last = if s.len() - i == 5 { s[i + 4] } else { 0.0 };
            let (a, b, c, e) = (s[i], s[i + 1], s[i + 2], s[i + 3]);
            if horizontal {
                self.curve([a, 0.0, b, c, last, e])
            } else {
                self.curve([0.0, a, b, c, e, last])
            }
            horizontal = !horizontal;
            i += 4;
        }
    }

    /// flex, hflex, hflex1, flex1: two curves (drawn plainly; flex depth is a hint).
    fn flex(&mut self, op: u16, s: &[f32]) {
        let g = |k: usize| s.get(k).copied().unwrap_or(0.0);
        let (c1, c2) = match op {
            0x0c23 => (
                [g(0), g(1), g(2), g(3), g(4), g(5)],
                [g(6), g(7), g(8), g(9), g(10), g(11)],
            ),
            0x0c22 => (
                [g(0), 0.0, g(1), g(2), g(3), 0.0],
                [g(4), 0.0, g(5), -g(2), g(6), 0.0],
            ),
            0x0c24 => (
                [g(0), g(1), g(2), g(3), g(4), 0.0],
                [g(5), 0.0, g(6), g(7), g(8), -(g(1) + g(3) + g(7))],
            ),
            _ => {
                let (dx, dy) = (
                    (0..5).map(|k| g(2 * k)).sum::<f32>(),
                    (0..5).map(|k| g(2 * k + 1)).sum::<f32>(),
                );
                let last = if dx.abs() > dy.abs() {
                    (g(10), -dy)
                } else {
                    (-dx, g(10))
                };
                (
                    [g(0), g(1), g(2), g(3), g(4), g(5)],
                    [g(6), g(7), g(8), g(9), last.0, last.1],
                )
            }
        };
        self.curve(c1);
        self.curve(c2);
    }

    fn line(&mut self, dx: f32, dy: f32) {
        self.pen = (self.pen.0 + dx, self.pen.1 + dy);
        self.cur.push(Point {
            x: self.pen.0,
            y: self.pen.1,
            on: true,
        });
    }

    /// A cubic from the pen through relative control points, cut into lines: n parts
    /// stray at most 3/4 · max(|p0 − 2p1 + p2|, |p1 − 2p2 + p3|) / n².
    fn curve(&mut self, [a, b, c, d, e, f]: [f32; 6]) {
        let p0 = self.pen;
        let p1 = (p0.0 + a, p0.1 + b);
        let p2 = (p1.0 + c, p1.1 + d);
        let p3 = (p2.0 + e, p2.1 + f);
        let dev = |q0: (f32, f32), q1: (f32, f32), q2: (f32, f32)| {
            (q0.0 - 2.0 * q1.0 + q2.0).hypot(q0.1 - 2.0 * q1.1 + q2.1)
        };
        let m = dev(p0, p1, p2).max(dev(p1, p2, p3));
        let n = ((0.75 * m / self.tol).sqrt().ceil() as usize).clamp(1, 64);
        for k in 1..=n {
            let t = k as f32 / n as f32;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let x = w0 * p0.0 + w1 * p1.0 + w2 * p2.0 + w3 * p3.0;
            let y = w0 * p0.1 + w1 * p1.1 + w2 * p2.1 + w3 * p3.1;
            self.cur.push(Point { x, y, on: true });
        }
        self.pen = p3;
    }

    fn close(&mut self) {
        if self.cur.len() > 1 {
            self.out.push(std::mem::take(&mut self.cur));
        }
        self.cur.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_dicts() {
        // 139 → 0, 28 x x → 16-bit, 247 0 → 108, 251 0 → -108; op 17 (CharStrings).
        let d = [139, 28, 0x01, 0x00, 247, 0, 251, 0, 17];
        assert_eq!(dict(&d).unwrap(), [(17, vec![0, 256, 108, -108])]);
    }

    #[test]
    fn a_charstring_draws_a_square_and_skips_the_width() {
        // width 50, rmoveto 0 0, hlineto 100 (h) 100 (v) -100 (h), endchar.
        let n = |v: i32| (v + 139) as u8;
        let code = [n(50), n(0), n(0), 21, 247, 0, 247, 0, 251, 0, 6, 14];
        let mut run = Run {
            d: &[],
            globals: Index::default(),
            locals: Index::default(),
            tol: 0.1,
            stack: Vec::new(),
            pen: (0.0, 0.0),
            stems: 0,
            width_done: false,
            out: Vec::new(),
            cur: Vec::new(),
        };
        assert!(run.exec(&code, 0).unwrap());
        run.close();
        let xy: Vec<_> = run.out[0].iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(xy, [(0.0, 0.0), (108.0, 0.0), (108.0, 108.0), (0.0, 108.0)]);
    }
}
