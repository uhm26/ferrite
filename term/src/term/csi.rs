//! CSI sequences: cursor movement, erasing, scrolling, modes and reports.
//! Reference: xterm's ctlseqs (<https://invisible-island.net/xterm/ctlseqs/ctlseqs.html>).

use super::{CursorShape, Mouse, Term};
use crate::vt::Params;

impl Term {
    pub(super) fn csi_dispatch(&mut self, p: &Params, inter: &[u8], action: u8) {
        match inter {
            [] => {
                if !self.csi_plain(p, action) {
                    self.csi_report(p, action);
                }
            }
            [b'?'] => match action {
                b'h' | b'l' => (0..p.len()).for_each(|i| self.dec_mode(p.get(i), action == b'h')),
                b'n' if p.get(0) == 6 => self.report_cursor(true),
                _ => {}
            },
            [b'?', b'$'] if action == b'p' => self.report_dec_mode(p.get(0)),
            [b'$'] if action == b'p' => self.reply(&format!("\x1b[{};0$y", p.get(0))),
            [b'>'] => match action {
                b'c' => self.reply("\x1b[>1;10;0c"),
                b'q' => self.reply(concat!("\x1bP>|term ", env!("CARGO_PKG_VERSION"), "\x1b\\")),
                _ => {} // `>m` (modifyOtherKeys) and friends: unsupported
            },
            [b' '] if action == b'q' => self.set_cursor_style(p.get(0)),
            [b'!'] if action == b'p' => self.soft_reset(),
            _ => {} // `?u` (kitty keyboard query): unsupported, so no reply
        }
    }

    /// Plain CSI that edit or move. Returns false if `action` isn't one of them.
    fn csi_plain(&mut self, p: &Params, action: u8) -> bool {
        let n = usize::from(p.arg(0, 1));
        let d = n as isize;
        match action {
            b'@' => self.insert_chars(n),
            b'A' => self.move_rows(-d),
            b'B' | b'e' => self.move_rows(d),
            b'C' | b'a' => self.move_cols(d),
            b'D' => self.move_cols(-d),
            b'E' => {
                self.move_rows(d);
                self.carriage_return();
            }
            b'F' => {
                self.move_rows(-d);
                self.carriage_return();
            }
            b'G' | b'`' => self.goto_col(n - 1),
            b'H' | b'f' => {
                let col = usize::from(p.arg(1, 1));
                self.goto(n - 1, col - 1);
            }
            b'I' => self.tab(n),
            b'J' => self.erase_display(p.get(0)),
            b'K' => self.erase_line(p.get(0)),
            b'L' => self.insert_lines(n, true),
            b'M' => self.insert_lines(n, false),
            b'P' => self.delete_chars(n),
            b'S' => self.scroll_up(n),
            b'T' => self.scroll_down(n),
            b'X' => self.erase_chars(n),
            b'Z' => self.back_tab(n),
            b'b' => self.repeat(n),
            b'd' => self.goto_row(n - 1),
            b'g' => self.clear_tabs(p.get(0)),
            b'h' | b'l' => (0..p.len()).for_each(|i| self.ansi_mode(p.get(i), action == b'h')),
            b'm' => self.sgr(p),
            b'r' => {
                let bot = usize::from(p.arg(1, self.rows as u16));
                self.set_scroll_region(n - 1, bot - 1);
            }
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(),
            _ => return false,
        }
        true
    }

    fn csi_report(&mut self, p: &Params, action: u8) {
        match (action, p.get(0)) {
            (b'c', 0) => self.reply("\x1b[?62;22c"), // VT220 with ANSI color
            (b'n', 5) => self.reply("\x1b[0n"),
            (b'n', 6) => self.report_cursor(false),
            (b't', 14) => {
                let (w, h) = self.cell_px;
                let (w, h) = (usize::from(w) * self.cols, usize::from(h) * self.rows);
                self.reply(&format!("\x1b[4;{h};{w}t"));
            }
            (b't', 16) => {
                let (w, h) = self.cell_px;
                self.reply(&format!("\x1b[6;{h};{w}t"));
            }
            (b't', 18) => self.reply(&format!("\x1b[8;{};{}t", self.rows, self.cols)),
            _ => {}
        }
    }

    fn goto_col(&mut self, col: usize) {
        self.cursor.col = col.min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn goto_row(&mut self, row: usize) {
        let col = self.cursor.col;
        self.goto(row, col);
    }

    fn clear_tabs(&mut self, mode: u16) {
        match mode {
            0 => self.tabs[self.cursor.col] = false,
            3 => self.tabs.fill(false),
            _ => {}
        }
    }

    fn report_cursor(&mut self, private: bool) {
        let row = if self.modes.origin {
            self.cursor.row.saturating_sub(self.top)
        } else {
            self.cursor.row
        };
        let q = if private { "?" } else { "" };
        self.reply(&format!("\x1b[{q}{};{}R", row + 1, self.cursor.col + 1));
    }

    fn ansi_mode(&mut self, mode: u16, on: bool) {
        match mode {
            4 => self.modes.insert = on,
            20 => self.modes.newline = on,
            _ => {}
        }
    }

    fn dec_mode(&mut self, mode: u16, on: bool) {
        let m = &mut self.modes;
        match mode {
            1 => m.app_cursor = on,
            6 => {
                m.origin = on;
                self.goto(0, 0);
            }
            7 => m.autowrap = on,
            25 => m.show_cursor = on,
            1000 => m.mouse = if on { Mouse::Click } else { Mouse::Off },
            1002 => m.mouse = if on { Mouse::Drag } else { Mouse::Off },
            1003 => m.mouse = if on { Mouse::Motion } else { Mouse::Off },
            1004 => m.focus_events = on,
            1006 => m.mouse_sgr = on,
            1007 => m.alt_scroll = on,
            2004 => m.bracketed_paste = on,
            2026 => m.sync = on,
            47 | 1047 => self.set_alt_screen(on, false),
            1048 if on => self.save_cursor(),
            1048 => self.restore_cursor(),
            1049 => self.set_alt_screen(on, true),
            _ => {}
        }
    }

    /// DECRQM: 1 = set, 2 = reset, 0 = unknown. Apps use it to find features (2026).
    fn report_dec_mode(&mut self, mode: u16) {
        let m = &self.modes;
        let state = match mode {
            1 => m.app_cursor,
            7 => m.autowrap,
            25 => m.show_cursor,
            1000 => m.mouse == Mouse::Click,
            1002 => m.mouse == Mouse::Drag,
            1003 => m.mouse == Mouse::Motion,
            1004 => m.focus_events,
            1006 => m.mouse_sgr,
            1049 | 1047 | 47 => self.alt,
            2004 => m.bracketed_paste,
            2026 => m.sync,
            _ => {
                self.reply(&format!("\x1b[?{mode};0$y"));
                return;
            }
        };
        let v = if state { 1 } else { 2 };
        self.reply(&format!("\x1b[?{mode};{v}$y"));
    }

    /// DECSCUSR: 0/1/2 block, 3/4 underline, 5/6 beam (blink is ignored).
    fn set_cursor_style(&mut self, style: u16) {
        self.shape = match style {
            3 | 4 => CursorShape::Underline,
            5 | 6 => CursorShape::Beam,
            _ => CursorShape::Block,
        };
    }

    /// DECSTR: modes and pen back to defaults, screen content stays.
    fn soft_reset(&mut self) {
        self.modes = super::Modes::default();
        self.pen = Default::default();
        self.top = 0;
        self.bot = self.rows - 1;
        self.charsets = [false; 2];
        self.gl = 0;
        self.shape = CursorShape::Block;
    }
}
