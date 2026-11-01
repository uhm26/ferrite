//! The terminal: what the shell's output means. Owns the primary and alternate
//! screens, scrollback, cursor, modes and palette. Knows nothing about drawing,
//! windows or keys; replies (cursor reports, DA, color queries) queue in `replies`.

pub mod cell;
mod csi;
mod dump;
mod edit;
mod esc;
mod reflow;
pub mod grid;
mod sgr;
#[cfg(test)]
mod tests;

use crate::color::Palette;
use crate::vt::{self, Params, Parser};
use cell::{Cell, Pen};
use grid::{Grid, History};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mouse {
    #[default]
    Off,
    /// 1000: presses and releases.
    Click,
    /// 1002: plus motion while a button is down.
    Drag,
    /// 1003: all motion.
    Motion,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Beam,
}

#[derive(Clone, Copy, Debug)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub autowrap: bool,
    pub show_cursor: bool,
    pub bracketed_paste: bool,
    pub insert: bool,
    pub origin: bool,
    pub newline: bool,
    pub focus_events: bool,
    /// 2026: the app is mid-frame; hold drawing until it ends.
    pub sync: bool,
    pub mouse: Mouse,
    pub mouse_sgr: bool,
    /// 1007: wheel sends arrow keys on the alternate screen.
    pub alt_scroll: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            app_cursor: false,
            app_keypad: false,
            autowrap: true,
            show_cursor: true,
            bracketed_paste: false,
            insert: false,
            origin: false,
            newline: false,
            focus_events: false,
            sync: false,
            mouse: Mouse::Off,
            mouse_sgr: false,
            alt_scroll: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    /// The last column was written; the next printable char wraps first.
    pub pending_wrap: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Saved {
    cursor: Cursor,
    pen: Pen,
    origin: bool,
    charsets: [bool; 2],
    gl: usize,
}

pub struct Term {
    parser: Parser,
    grid: Grid,
    /// The inactive screen (alternate when on primary, and the other way round).
    other: Grid,
    alt: bool,
    history: History,
    cursor: Cursor,
    saved: [Saved; 2],
    pen: Pen,
    /// Scroll region, inclusive rows.
    top: usize,
    bot: usize,
    tabs: Vec<bool>,
    /// G0/G1 use DEC line drawing; `gl` picks the active one (SI/SO).
    charsets: [bool; 2],
    gl: usize,
    last: char,
    cols: usize,
    rows: usize,
    /// Cell size in pixels, for CSI 14 t / 16 t.
    cell_px: (u16, u16),
    pub modes: Modes,
    pub shape: CursorShape,
    pub palette: Palette,
    base_palette: Palette,
    /// The shell has printed visible text. Until then the cursor is not drawn, so a
    /// new window doesn't flash it top-left while fish is still querying the terminal.
    pub started: bool,
    /// Something visible may have changed since the renderer last looked.
    pub dirty: bool,
    /// The palette changed: every cell must be redrawn.
    pub recolor: bool,
    pub title: Option<String>,
    pub bell: bool,
    /// Bytes to send back to the shell.
    pub replies: Vec<u8>,
    /// Rows scrolled back from the bottom (0 = live).
    pub view: usize,
}

impl Term {
    pub fn new(cols: usize, rows: usize, scrollback: usize, palette: Palette) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Term {
            parser: Parser::new(),
            grid: Grid::new(cols, rows),
            other: Grid::new(cols, rows),
            alt: false,
            history: History::new(scrollback),
            cursor: Cursor::default(),
            saved: [Saved::default(); 2],
            pen: Pen::default(),
            top: 0,
            bot: rows - 1,
            tabs: default_tabs(cols),
            charsets: [false; 2],
            gl: 0,
            last: ' ',
            cols,
            rows,
            cell_px: (0, 0),
            modes: Modes::default(),
            shape: CursorShape::default(),
            base_palette: palette.clone(),
            palette,
            started: false,
            dirty: true,
            recolor: true,
            title: None,
            bell: false,
            replies: Vec::new(),
            view: 0,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::take(&mut self.parser);
        parser.feed(self, bytes);
        self.parser = parser;
        self.dirty = true;
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn is_alt(&self) -> bool {
        self.alt
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Lines ever scrolled into history (for stable line numbers).
    pub fn history_pushed(&self) -> u64 {
        self.history.pushed
    }

    /// New colors (live theme change). Replaces colors an app set with OSC 4/10/11 too.
    pub fn set_palette(&mut self, p: Palette) {
        self.base_palette = p.clone();
        self.palette = p;
        self.recolor = true;
        self.dirty = true;
    }

    pub fn set_cell_px(&mut self, w: u16, h: u16) {
        self.cell_px = (w, h);
    }

    /// Row `y` of what is on screen now, honoring the scrollback view. May be
    /// shorter than `cols` (trimmed history); the rest is blank.
    pub fn visible_row(&self, y: usize) -> &[Cell] {
        let view = self.view.min(self.history.len());
        if y < view {
            let i = self.history.len() - view + y;
            return self.history.get(i).map_or(&[], |r| &r.cells);
        }
        &self.grid.row(y - view).cells
    }

    /// Scroll the view by `delta` rows (positive = back in time).
    pub fn scroll_view(&mut self, delta: isize) {
        let max = if self.alt { 0 } else { self.history.len() };
        let v = self.view.saturating_add_signed(delta).min(max);
        if v != self.view {
            self.view = v;
            self.dirty = true;
        }
    }

    pub fn reset_view(&mut self) {
        self.scroll_view(-(self.view as isize));
    }

    /// Line `abs` (history line numbers continue into the screen) as text.
    pub fn line_cells(&self, abs: u64) -> Option<(&[Cell], bool)> {
        let first = self.history.pushed - self.history.len() as u64;
        let i = abs.checked_sub(first)? as usize;
        if i < self.history.len() {
            return self.history.get(i).map(|r| (&r.cells[..], r.wrapped));
        }
        let y = i - self.history.len();
        (y < self.rows).then(|| {
            let r = self.grid.row(y);
            (&r.cells[..], r.wrapped)
        })
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        let (primary, alt) = if self.alt {
            (&mut self.other, &mut self.grid)
        } else {
            (&mut self.grid, &mut self.other)
        };
        let cur = if self.alt {
            &mut self.saved[0].cursor
        } else {
            &mut self.cursor
        };
        reflow::reflow(primary, &mut self.history, cur, (cols, rows));
        alt.set_cols(cols); // full-screen apps redraw themselves
        alt.truncate(rows);
        alt.grow(rows);
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bot = rows - 1;
        self.tabs = default_tabs(cols);
        let [s0, s1] = &mut self.saved;
        for c in [&mut self.cursor, &mut s0.cursor, &mut s1.cursor] {
            c.row = c.row.min(rows - 1);
            c.col = c.col.min(cols - 1);
            c.pending_wrap = false;
        }
        self.view = 0;
        self.dirty = true;
    }

    /// The pen's current bg for erasing.
    fn blank(&self) -> Cell {
        Cell::erased(&self.pen)
    }

    fn reply(&mut self, s: &str) {
        self.replies.extend_from_slice(s.as_bytes());
    }

    fn full_reset(&mut self) {
        let fresh = Term::new(self.cols, self.rows, 0, self.base_palette.clone());
        let history = std::mem::replace(&mut self.history, History::new(0));
        let cell_px = self.cell_px;
        *self = Term {
            history,
            cell_px,
            replies: std::mem::take(&mut self.replies),
            ..fresh
        };
    }
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c % 8 == 0).collect()
}

impl vt::Perform for Term {
    fn print_ascii(&mut self, s: &[u8]) {
        self.started = true;
        self.put_ascii(s);
    }

    fn print(&mut self, c: char) {
        self.started = true;
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        self.control(byte);
    }

    fn csi(&mut self, p: &Params, inter: &[u8], action: u8) {
        self.csi_dispatch(p, inter, action);
    }

    fn esc(&mut self, inter: &[u8], action: u8) {
        self.esc_dispatch(inter, action);
    }

    fn osc(&mut self, data: &[u8], bel: bool) {
        self.osc_dispatch(data, bel);
    }
}
