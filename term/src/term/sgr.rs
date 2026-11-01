//! SGR (`CSI … m`): text attributes and colors, in both the `;` and `:` forms
//! (`38;2;r;g;b`, `38:2::r:g:b`, `38;5;n`, `4:3` curly underline).

use super::Term;
use super::cell::{Color, Flags, Pen};
use crate::vt::Params;

impl Term {
    pub(super) fn sgr(&mut self, p: &Params) {
        if p.is_empty() {
            self.pen = Pen::default();
            return;
        }
        let mut groups = p.groups();
        while let Some(g) = groups.next() {
            let code = g[0];
            match code {
                38 | 48 | 58 => {
                    let color = if g.len() > 1 {
                        extended(&g[1..])
                    } else {
                        extended_split(&mut groups)
                    };
                    match (code, color) {
                        (38, Some(c)) => self.pen.fg = c,
                        (48, Some(c)) => self.pen.bg = c,
                        _ => {} // 58: underline color, not stored (v1)
                    }
                }
                4 => underline(&mut self.pen.flags, g.get(1).copied().unwrap_or(1)),
                _ => attr(&mut self.pen, code),
            }
        }
    }
}

fn underline(f: &mut Flags, style: u16) {
    f.set(Flags::ANY_UL, false);
    match style {
        0 => {}
        2 => f.set(Flags::DOUBLE_UL, true),
        3 => f.set(Flags::CURLY_UL, true),
        _ => f.set(Flags::UNDERLINE, true), // single, dotted, dashed
    }
}

fn attr(pen: &mut Pen, code: u16) {
    let f = &mut pen.flags;
    match code {
        0 => *pen = Pen::default(),
        1 => f.set(Flags::BOLD, true),
        2 => f.set(Flags::DIM, true),
        3 => f.set(Flags::ITALIC, true),
        7 => f.set(Flags::REVERSE, true),
        8 => f.set(Flags::HIDDEN, true),
        9 => f.set(Flags::STRIKE, true),
        21 => underline(f, 2),
        22 => f.set(Flags::BOLD | Flags::DIM, false),
        23 => f.set(Flags::ITALIC, false),
        24 => f.set(Flags::ANY_UL, false),
        27 => f.set(Flags::REVERSE, false),
        28 => f.set(Flags::HIDDEN, false),
        29 => f.set(Flags::STRIKE, false),
        30..=37 => pen.fg = Color::index((code - 30) as u8),
        39 => pen.fg = Color::DEFAULT,
        40..=47 => pen.bg = Color::index((code - 40) as u8),
        49 => pen.bg = Color::DEFAULT,
        90..=97 => pen.fg = Color::index((code - 90 + 8) as u8),
        100..=107 => pen.bg = Color::index((code - 100 + 8) as u8),
        _ => {} // blink, fonts, overline…: ignored
    }
}

/// `:` form, after the 38/48: `5:n`, `2:r:g:b` or `2:cs:r:g:b`.
fn extended(sub: &[u16]) -> Option<Color> {
    match sub {
        [5, n, ..] => Some(Color::index(*n as u8)),
        [2, _, r, g, b, ..] => Some(rgb(*r, *g, *b)),
        [2, r, g, b] => Some(rgb(*r, *g, *b)),
        _ => None,
    }
}

/// `;` form: the color's values are the next groups.
fn extended_split<'a>(groups: &mut impl Iterator<Item = &'a [u16]>) -> Option<Color> {
    let mut next = || groups.next().map(|g| g[0]);
    match next()? {
        5 => next().map(|n| Color::index(n as u8)),
        2 => {
            let (r, g, b) = (next()?, next()?, next()?);
            Some(rgb(r, g, b))
        }
        _ => None,
    }
}

fn rgb(r: u16, g: u16, b: u16) -> Color {
    Color::rgb(r as u8, g as u8, b as u8)
}
