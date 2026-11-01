//! The terminal as VT bytes: scrollback, screen, alternate screen, cursor, modes
//! and pen. Feeding the result to a fresh [`Term`] of the same size rebuilds the
//! state; that is how a restart in place keeps every tab's content.

use super::cell::{Cell, Color, Flags, Kind};
use super::grid::Row;
use super::{CursorShape, Mouse, Term};

/// Attributes that matter for SGR (not the wide-char layout bits).
type Style = (Color, Color, u32);

impl Term {
    pub fn dump(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut pen = None;
        let primary = if self.alt { &self.other } else { &self.grid };
        let history = (0..self.history.len()).filter_map(|i| self.history.get(i));
        let rows: Vec<&Row> = history
            .chain((0..self.rows).map(|y| primary.row(y)))
            .collect();
        for (i, r) in rows.iter().enumerate() {
            row(&mut out, &r.cells[..r.cells.len().min(self.cols)], &mut pen);
            if i + 1 == rows.len() {
                break;
            }
            if r.wrapped {
                // Fill to the edge so the replay wraps here too (and marks the row).
                let cells = &r.cells[..r.cells.len().min(self.cols)];
                let used = cells.iter().rposition(|c| *c != Cell::BLANK).map_or(0, |i| i + 1);
                sgr(&mut out, (Color::DEFAULT, Color::DEFAULT, 0));
                pen = None;
                out.resize(out.len() + self.cols.saturating_sub(used), b' ');
            } else {
                out.extend_from_slice(b"\r\n");
            }
        }
        let main_cursor = if self.alt {
            self.saved[0].cursor
        } else {
            self.cursor
        };
        if !self.alt {
            self.region(&mut out);
        }
        cup(&mut out, main_cursor.row, main_cursor.col);
        if self.alt {
            out.extend_from_slice(b"\x1b[?1049h");
            for y in 0..self.rows {
                cup(&mut out, y, 0);
                row(&mut out, &self.grid.row(y).cells, &mut pen);
            }
            self.region(&mut out);
            cup(&mut out, self.cursor.row, self.cursor.col);
        }
        self.modes_bytes(&mut out);
        let p = self.pen;
        sgr(&mut out, (p.fg, p.bg, p.flags.0));
        out
    }

    fn region(&self, out: &mut Vec<u8>) {
        if (self.top, self.bot) != (0, self.rows - 1) {
            out.extend_from_slice(format!("\x1b[{};{}r", self.top + 1, self.bot + 1).as_bytes());
        }
    }

    fn modes_bytes(&self, out: &mut Vec<u8>) {
        let m = &self.modes;
        let on = [
            (m.app_cursor, "\x1b[?1h"),
            (!m.autowrap, "\x1b[?7l"),
            (!m.show_cursor, "\x1b[?25l"),
            (m.bracketed_paste, "\x1b[?2004h"),
            (m.mouse == Mouse::Click, "\x1b[?1000h"),
            (m.mouse == Mouse::Drag, "\x1b[?1002h"),
            (m.mouse == Mouse::Motion, "\x1b[?1003h"),
            (m.mouse_sgr, "\x1b[?1006h"),
            (m.focus_events, "\x1b[?1004h"),
            (!m.alt_scroll, "\x1b[?1007l"),
            (m.app_keypad, "\x1b="),
            (m.insert, "\x1b[4h"),
            (m.newline, "\x1b[20h"),
            (self.shape == CursorShape::Underline, "\x1b[4 q"),
            (self.shape == CursorShape::Beam, "\x1b[6 q"),
        ];
        for (set, seq) in on {
            if set {
                out.extend_from_slice(seq.as_bytes());
            }
        }
    }
}

fn cup(out: &mut Vec<u8>, row: usize, col: usize) {
    out.extend_from_slice(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
}

/// One row's text, trailing blanks left out, SGR only where the style changes.
fn row(out: &mut Vec<u8>, cells: &[Cell], pen: &mut Option<Style>) {
    let end = cells
        .iter()
        .rposition(|c| *c != Cell::BLANK)
        .map_or(0, |i| i + 1);
    let layout = Flags::WIDE | Flags::SPACER;
    for c in &cells[..end] {
        if c.flags.has(Flags::SPACER) {
            continue;
        }
        let style = (c.fg, c.bg, c.flags.0 & Flags::TERM_MASK & !layout);
        if *pen != Some(style) {
            sgr(out, style);
            *pen = Some(style);
        }
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.ch.encode_utf8(&mut buf).as_bytes());
    }
}

fn sgr(out: &mut Vec<u8>, (fg, bg, flags): Style) {
    let mut s = String::from("\x1b[0");
    let f = Flags(flags);
    for (bit, code) in [
        (Flags::BOLD, ";1"),
        (Flags::DIM, ";2"),
        (Flags::ITALIC, ";3"),
        (Flags::UNDERLINE, ";4"),
        (Flags::DOUBLE_UL, ";4:2"),
        (Flags::CURLY_UL, ";4:3"),
        (Flags::REVERSE, ";7"),
        (Flags::HIDDEN, ";8"),
        (Flags::STRIKE, ";9"),
    ] {
        if f.has(bit) {
            s.push_str(code);
        }
    }
    color(&mut s, fg, 30);
    color(&mut s, bg, 40);
    s.push('m');
    out.extend_from_slice(s.as_bytes());
}

/// `base` 30 = foreground, 40 = background.
fn color(s: &mut String, c: Color, base: u16) {
    use std::fmt::Write;
    let _ = match c.kind() {
        Kind::Default => Ok(()),
        Kind::Index(i) if i < 8 => write!(s, ";{}", base + u16::from(i)),
        Kind::Index(i) if i < 16 => write!(s, ";{}", base + 60 + u16::from(i - 8)),
        Kind::Index(i) => write!(s, ";{};5;{i}", base + 8),
        Kind::Rgb(v) => write!(
            s,
            ";{};2;{};{};{}",
            base + 8,
            v >> 16,
            v >> 8 & 0xff,
            v & 0xff
        ),
    };
}

#[cfg(test)]
mod tests {
    use super::super::Term;
    use crate::config::Config;

    fn term(cols: usize, rows: usize) -> Term {
        crate::sys::init_locale();
        Term::new(cols, rows, 100, Config::default().palette())
    }

    fn same(a: &Term, b: &Term) {
        assert_eq!(a.history_len(), b.history_len());
        for i in 0..a.history_len() as u64 {
            let first = a.history_pushed() - a.history_len() as u64;
            let line = |t: &Term, n| t.line_cells(n).map(|(c, w)| (c.to_vec(), w));
            let fb = b.history_pushed() - b.history_len() as u64;
            assert_eq!(
                line(a, first + i).map(|x| x.0),
                line(b, fb + i).map(|x| x.0)
            );
        }
        for y in 0..a.rows() {
            let pad = |t: &Term| {
                let mut r = t.visible_row(y).to_vec();
                r.resize(t.cols(), super::Cell::BLANK);
                r
            };
            assert_eq!(pad(a), pad(b), "row {y}");
        }
        let (ca, cb) = (a.cursor(), b.cursor());
        assert_eq!((ca.row, ca.col), (cb.row, cb.col));
        assert_eq!(a.is_alt(), b.is_alt());
    }

    fn roundtrip(input: &[u8]) {
        let mut a = term(12, 4);
        a.feed(input);
        let mut b = term(12, 4);
        b.feed(&a.dump());
        same(&a, &b);
        let (ma, mb) = (a.modes, b.modes);
        assert_eq!(
            (ma.app_cursor, ma.bracketed_paste, ma.mouse, ma.mouse_sgr),
            (mb.app_cursor, mb.bracketed_paste, mb.mouse, mb.mouse_sgr)
        );
    }

    #[test]
    fn text_colors_scrollback() {
        roundtrip(b"one\r\n\x1b[1;31mtwo\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[48;5;200m bg\x1b[0m\r\nthree\r\nfour\r\nfive\r\nsix");
    }

    #[test]
    fn wrapped_wide_and_cursor() {
        roundtrip("abcdefghijklmnop 漢字 x\x1b[2;3H".as_bytes());
    }

    #[test]
    fn wrapped_row_ending_in_space() {
        roundtrip(b"aaaaaaaaaaa bbbb");
    }

    #[test]
    fn alt_screen_and_modes() {
        roundtrip(b"main\r\nshell\x1b[?1049h\x1b[?1h\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[3;2Hvim\x1b[4:3mx");
    }
}
