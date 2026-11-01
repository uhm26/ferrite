//! Writing text and moving things: printing, wrapping, line feeds, scrolling,
//! erasing, inserting and deleting, cursor moves, screen switches.

use super::cell::{Cell, Flags};
use super::{Saved, Term};
use crate::sys::char_width;

/// DEC special graphics for `0x60..=0x7e` (ESC ( 0), as used by tmux, mc, dialog.
const LINE_DRAWING: [char; 31] = [
    '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻', '─', '⎼',
    '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
];

impl Term {
    /// Printable ASCII in bulk: the hot path for most output.
    pub(super) fn put_ascii(&mut self, s: &[u8]) {
        if self.charsets[self.gl] || self.modes.insert {
            s.iter().for_each(|&b| self.put_char(b as char));
            return;
        }
        let Some(&last) = s.last() else { return };
        let mut s = s;
        while !s.is_empty() {
            if self.cursor.pending_wrap {
                self.wrap();
            }
            let (col, cols) = (self.cursor.col, self.cols);
            let n = (cols - col).min(s.len());
            self.clear_wide_overlap(col, n);
            let pen = self.pen;
            let cells = &mut self.grid.row_mut(self.cursor.row).cells[col..col + n];
            for (cell, &b) in cells.iter_mut().zip(&s[..n]) {
                *cell = pen.cell(b as char);
            }
            s = &s[n..];
            if col + n < cols {
                self.cursor.col = col + n;
            } else if self.modes.autowrap {
                self.cursor.col = cols - 1;
                self.cursor.pending_wrap = true;
            } else {
                self.cursor.col = cols - 1;
                if let Some(&b) = s.last() {
                    self.grid.row_mut(self.cursor.row).cells[cols - 1] = pen.cell(b as char);
                }
                break;
            }
        }
        self.last = last as char;
    }

    pub(super) fn put_char(&mut self, c: char) {
        let c = match c {
            '\u{60}'..='\u{7e}' if self.charsets[self.gl] => LINE_DRAWING[c as usize - 0x60],
            c => c,
        };
        let width = char_width(c);
        if width == 0 {
            return; // combining marks are not stored yet (v1)
        }
        if self.cursor.pending_wrap {
            self.wrap();
        }
        if width == 2 && self.cursor.col + 1 >= self.cols {
            if !self.modes.autowrap || self.cols < 2 {
                return;
            }
            self.wrap(); // a wide char never splits across rows
        }
        if self.modes.insert {
            self.insert_chars(width);
        }
        let col = self.cursor.col;
        self.clear_wide_overlap(col, width);
        let mut cell = self.pen.cell(c);
        let row = &mut self.grid.row_mut(self.cursor.row).cells;
        if width == 2 {
            cell.flags.set(Flags::WIDE, true);
            row[col] = cell;
            let mut spacer = self.pen.cell(' ');
            spacer.flags.set(Flags::SPACER, true);
            row[col + 1] = spacer;
        } else {
            row[col] = cell;
        }
        self.advance(width);
        self.last = c;
    }

    fn advance(&mut self, width: usize) {
        if self.cursor.col + width < self.cols {
            self.cursor.col += width;
        } else {
            self.cursor.col = self.cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        }
    }

    /// Writing `n` cells at `col` may cut a wide char in half; blank the other half.
    fn clear_wide_overlap(&mut self, col: usize, n: usize) {
        let cells = &mut self.grid.row_mut(self.cursor.row).cells;
        if col > 0 && cells[col].flags.has(Flags::SPACER) {
            cells[col - 1] = Cell::BLANK;
        }
        let end = col + n;
        if end < cells.len() && cells[end].flags.has(Flags::SPACER) {
            cells[end] = Cell::BLANK;
        }
    }

    fn wrap(&mut self) {
        self.grid.row_mut(self.cursor.row).wrapped = true;
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
        self.linefeed();
    }

    /// REP: repeat the last printed char.
    pub(super) fn repeat(&mut self, n: usize) {
        let c = self.last;
        for _ in 0..n.min(self.cols * self.rows) {
            self.put_char(c);
        }
    }

    pub(super) fn linefeed(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.bot {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        }
    }

    pub(super) fn reverse_index(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.top {
            self.scroll_down(1);
        } else {
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }
    }

    pub(super) fn scroll_up(&mut self, n: usize) {
        let blank = self.blank();
        let full = self.top == 0 && self.bot == self.rows - 1 && !self.alt;
        let before = self.history.pushed;
        let history = full.then_some(&mut self.history);
        self.grid.scroll_up((self.top, self.bot), n, blank, history);
        if self.view > 0 {
            // Keep the scrolled-back view on the same text.
            let moved = (self.history.pushed - before) as usize;
            self.view = (self.view + moved).min(self.history.len());
        }
    }

    pub(super) fn scroll_down(&mut self, n: usize) {
        let blank = self.blank();
        self.grid.scroll_down((self.top, self.bot), n, blank);
    }

    pub(super) fn carriage_return(&mut self) {
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    pub(super) fn backspace(&mut self) {
        self.cursor.col = self.cursor.col.saturating_sub(1);
        self.cursor.pending_wrap = false;
    }

    pub(super) fn tab(&mut self, n: usize) {
        for _ in 0..n {
            let next = (self.cursor.col + 1..self.cols).find(|&c| self.tabs[c]);
            self.cursor.col = next.unwrap_or(self.cols - 1);
        }
        self.cursor.pending_wrap = false;
    }

    pub(super) fn back_tab(&mut self, n: usize) {
        for _ in 0..n {
            let prev = (0..self.cursor.col).rev().find(|&c| self.tabs[c]);
            self.cursor.col = prev.unwrap_or(0);
        }
        self.cursor.pending_wrap = false;
    }

    /// CUP: 0-based target; origin mode makes rows relative to the scroll region.
    pub(super) fn goto(&mut self, row: usize, col: usize) {
        let (lo, hi) = if self.modes.origin {
            (self.top, self.bot)
        } else {
            (0, self.rows - 1)
        };
        self.cursor.row = (lo + row).min(hi);
        self.cursor.col = col.min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    /// Relative vertical move, stopping at the scroll region edge if inside it.
    pub(super) fn move_rows(&mut self, delta: isize) {
        let r = self.cursor.row;
        let (lo, hi) = if (self.top..=self.bot).contains(&r) {
            (self.top, self.bot)
        } else {
            (0, self.rows - 1)
        };
        self.cursor.row = r.saturating_add_signed(delta).clamp(lo, hi);
        self.cursor.pending_wrap = false;
    }

    pub(super) fn move_cols(&mut self, delta: isize) {
        self.cursor.col = self
            .cursor
            .col
            .saturating_add_signed(delta)
            .min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    /// ED: 0 = below, 1 = above, 2 = all, 3 = scrollback.
    pub(super) fn erase_display(&mut self, mode: u16) {
        let blank = self.blank();
        let row = self.cursor.row;
        match mode {
            0 => {
                self.erase_line(0);
                (row + 1..self.rows).for_each(|y| self.fill_row(y, blank));
            }
            1 => {
                self.erase_line(1);
                (0..row).for_each(|y| self.fill_row(y, blank));
            }
            2 => self.grid.clear(blank),
            3 => {
                self.history.clear();
                self.view = 0;
            }
            _ => {}
        }
    }

    fn fill_row(&mut self, y: usize, blank: Cell) {
        let row = self.grid.row_mut(y);
        row.cells.fill(blank);
        row.wrapped = false;
    }

    /// EL: 0 = to the right, 1 = to the left, 2 = whole line.
    pub(super) fn erase_line(&mut self, mode: u16) {
        let blank = self.blank();
        let col = self.cursor.col;
        let row = self.grid.row_mut(self.cursor.row);
        let range = match mode {
            0 => col..row.cells.len(),
            1 => 0..col + 1,
            2 => 0..row.cells.len(),
            _ => return,
        };
        if mode != 1 {
            row.wrapped = false;
        }
        row.cells[range].fill(blank);
        self.fix_row_edges();
    }

    /// After a partial erase, no wide char may be left without its other half.
    fn fix_row_edges(&mut self) {
        let cells = &mut self.grid.row_mut(self.cursor.row).cells;
        for i in 0..cells.len() {
            let wide = cells[i].flags.has(Flags::WIDE);
            let next_spacer = cells.get(i + 1).is_some_and(|c| c.flags.has(Flags::SPACER));
            if wide && !next_spacer {
                cells[i].flags.set(Flags::WIDE, false);
                cells[i].ch = ' ';
            }
            let prev_wide = i > 0 && cells[i - 1].flags.has(Flags::WIDE);
            if cells[i].flags.has(Flags::SPACER) && !prev_wide {
                cells[i].flags.set(Flags::SPACER, false);
            }
        }
    }

    /// ECH: blank `n` cells from the cursor, no shifting.
    pub(super) fn erase_chars(&mut self, n: usize) {
        let blank = self.blank();
        let col = self.cursor.col;
        let cells = &mut self.grid.row_mut(self.cursor.row).cells;
        let end = (col + n).min(cells.len());
        cells[col..end].fill(blank);
        self.fix_row_edges();
    }

    /// ICH: shift the rest of the line right by `n`.
    pub(super) fn insert_chars(&mut self, n: usize) {
        let blank = self.blank();
        let col = self.cursor.col;
        let cells = &mut self.grid.row_mut(self.cursor.row).cells;
        let n = n.min(cells.len() - col);
        cells[col..].rotate_right(n);
        cells[col..col + n].fill(blank);
        self.fix_row_edges();
    }

    /// DCH: shift the rest of the line left by `n`.
    pub(super) fn delete_chars(&mut self, n: usize) {
        let blank = self.blank();
        let col = self.cursor.col;
        let cells = &mut self.grid.row_mut(self.cursor.row).cells;
        let n = n.min(cells.len() - col);
        cells[col..].rotate_left(n);
        let len = cells.len();
        cells[len - n..].fill(blank);
        self.fix_row_edges();
    }

    /// IL / DL: only inside the scroll region.
    pub(super) fn insert_lines(&mut self, n: usize, insert: bool) {
        let row = self.cursor.row;
        if !(self.top..=self.bot).contains(&row) {
            return;
        }
        let blank = self.blank();
        if insert {
            self.grid.scroll_down((row, self.bot), n, blank);
        } else {
            self.grid.scroll_up((row, self.bot), n, blank, None);
        }
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    pub(super) fn set_scroll_region(&mut self, top: usize, bot: usize) {
        let bot = bot.min(self.rows - 1);
        if top < bot {
            self.top = top;
            self.bot = bot;
            self.goto(0, 0);
        }
    }

    pub(super) fn save_cursor(&mut self) {
        self.saved[usize::from(self.alt)] = Saved {
            cursor: self.cursor,
            pen: self.pen,
            origin: self.modes.origin,
            charsets: self.charsets,
            gl: self.gl,
        };
    }

    pub(super) fn restore_cursor(&mut self) {
        let s = self.saved[usize::from(self.alt)];
        self.cursor = s.cursor;
        self.cursor.row = self.cursor.row.min(self.rows - 1);
        self.cursor.col = self.cursor.col.min(self.cols - 1);
        self.pen = s.pen;
        self.modes.origin = s.origin;
        self.charsets = s.charsets;
        self.gl = s.gl;
    }

    /// 1049-style switch: save the cursor, swap screens, clear the alternate one.
    pub(super) fn set_alt_screen(&mut self, on: bool, save: bool) {
        if on == self.alt {
            return;
        }
        if on && save {
            self.save_cursor();
        }
        std::mem::swap(&mut self.grid, &mut self.other);
        self.alt = on;
        if on {
            self.grid.clear(Cell::BLANK);
        } else if save {
            self.restore_cursor();
        }
        self.top = 0;
        self.bot = self.rows - 1;
        self.view = 0;
    }
}
