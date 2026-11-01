//! CPU renderer. Each buffer remembers the cells it shows ([`Shown`]); a frame
//! compares the terminal against that and redraws only cells that differ, then
//! reports the changed rectangles as damage. Cursor and selection are folded into
//! the compared cells as extra flag bits, so they need no special tracking.

pub mod draw;
mod tabbar;

use crate::color::{self, Theme};
use crate::font::{Font, GlyphCache};
use crate::term::cell::{Cell, Flags};
use crate::term::{CursorShape, Term};
use draw::{Canvas, Rect};

// Renderer-only flag bits, above `Flags::TERM_MASK`.
const CURSOR: u32 = 1 << 16;
const CURSOR_UNDERLINE: u32 = 1 << 17;
const CURSOR_BEAM: u32 = 1 << 18;
const CURSOR_HOLLOW: u32 = 1 << 19;
const SELECTED: u32 = 1 << 20;

/// Never equal to a real cell: marks a [`Shown`] slot as "draw me".
const STALE: Cell = Cell {
    ch: '\u{10ffff}',
    ..Cell::BLANK
};

/// What one buffer currently shows.
#[derive(Default)]
pub struct Shown {
    cells: Vec<Cell>,
    cols: usize,
    rows: usize,
    /// Tab labels and active tab the bar was drawn with.
    bar: Option<(Vec<String>, usize)>,
}

impl Shown {
    /// Forget everything: the next frame redraws the whole buffer.
    pub fn invalidate(&mut self) {
        self.cols = 0;
        self.bar = None;
    }
}

/// A text selection in line numbers that survive scrolling, inclusive, ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: (u64, usize),
    pub end: (u64, usize),
}

impl Selection {
    fn contains(&self, line: u64, col: usize) -> bool {
        (line, col) >= self.start && (line, col) <= self.end
    }
}

pub struct View<'a> {
    pub focused: bool,
    pub selection: Option<Selection>,
    /// Tab titles; the bar shows when there are two or more.
    pub tabs: &'a [String],
    pub active_tab: usize,
}

pub struct Renderer {
    pub font: Font,
    cache: GlyphCache,
    pub pad: usize,
    /// Pixels above the grid taken by the tab bar (0 = no bar).
    pub top: usize,
    theme: Theme,
    row: Vec<Cell>,
}

impl Renderer {
    pub fn new(font: Font, pad: usize, theme: Theme) -> Self {
        Renderer {
            font,
            cache: GlyphCache::default(),
            pad,
            top: 0,
            theme,
            row: Vec::new(),
        }
    }

    /// Columns and rows that fit in `w`×`h` pixels.
    pub fn grid_for(&self, w: usize, h: usize) -> (usize, usize) {
        let inner = |v: usize| v.saturating_sub(2 * self.pad);
        let cols = inner(w) / self.font.cell_w;
        let rows = inner(h.saturating_sub(self.top)) / self.font.cell_h;
        (cols.max(1), rows.max(1))
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn set_font(&mut self, font: Font) {
        self.font = font;
        self.cache.clear();
    }

    pub fn set_font_size(&mut self, pt: f32) {
        self.font.resize(pt);
        self.cache.clear();
    }

    pub fn glyph_bytes(&self) -> usize {
        self.cache.bytes()
    }

    /// Bring `canvas` (described by `shown`) up to date with `term`. Returns damage.
    pub fn draw(
        &mut self,
        term: &Term,
        view: &View,
        canvas: &mut Canvas,
        shown: &mut Shown,
    ) -> Vec<Rect> {
        let (cols, rows) = (term.cols(), term.rows());
        let mut damage = Vec::new();
        if shown.cols != cols || shown.rows != rows {
            let all = Rect {
                x: 0,
                y: 0,
                w: canvas.w,
                h: canvas.h,
            };
            // Every cell gets drawn below; paint only the margins around the grid.
            let (x1, y1) = (self.pad + cols * self.font.cell_w, self.pad + self.top + rows * self.font.cell_h);
            let (y0, (w, h)) = (self.pad + self.top, (canvas.w, canvas.h));
            for r in [
                Rect { x: 0, y: 0, w, h: y0 },
                Rect { x: 0, y: y1, w, h: h.saturating_sub(y1) },
                Rect { x: 0, y: y0, w: self.pad, h: y1.saturating_sub(y0) },
                Rect { x: x1, y: y0, w: w.saturating_sub(x1), h: y1.saturating_sub(y0) },
            ] {
                canvas.fill(r, term.palette.bg_px());
            }
            shown.cells = vec![STALE; cols * rows];
            (shown.cols, shown.rows) = (cols, rows);
            shown.bar = None;
            damage.push(all);
        }
        if let Some(r) = self.tab_bar(term, view, canvas, shown) {
            damage.push(r);
        }
        let marks = Marks::new(term, view);
        for y in 0..rows {
            self.fill_row(term, &marks, y);
            let old = &mut shown.cells[y * cols..(y + 1) * cols];
            if let Some(r) = self.draw_row(term, canvas, old, y) {
                damage.push(r);
            }
        }
        if damage
            .first()
            .is_some_and(|r| r.w == canvas.w && r.h == canvas.h)
        {
            damage.truncate(1);
        }
        damage
    }

    /// The cells of visible row `y` with cursor/selection marks, padded to `cols`.
    fn fill_row(&mut self, term: &Term, marks: &Marks, y: usize) {
        let src = term.visible_row(y);
        self.row.clear();
        self.row
            .extend_from_slice(&src[..src.len().min(term.cols())]);
        self.row.resize(term.cols(), Cell::BLANK);
        let line = marks.first_line + y as u64;
        if let Some(sel) = marks.selection
            && (sel.start.0..=sel.end.0).contains(&line) // rows outside: no per-cell checks
        {
            for (x, c) in self.row.iter_mut().enumerate() {
                if sel.contains(line, x) {
                    c.flags.0 |= SELECTED;
                }
            }
        }
        if let Some((cy, cx)) = marks.cursor
            && cy == y
        {
            // On the second half of a wide char, mark the first half.
            let cx = if cx > 0 && self.row[cx].flags.has(Flags::SPACER) {
                cx - 1
            } else {
                cx
            };
            self.row[cx].flags.0 |= marks.cursor_bits;
        }
    }

    /// Redraw cells of row `y` that differ from `old`; update `old`. Returns damage.
    fn draw_row(
        &mut self,
        term: &Term,
        canvas: &mut Canvas,
        old: &mut [Cell],
        y: usize,
    ) -> Option<Rect> {
        let cols = self.row.len();
        let (mut lo, mut hi) = (usize::MAX, 0);
        let mut x = 0;
        while x < cols {
            let cell = self.row[x];
            let wide = cell.flags.has(Flags::WIDE) && x + 1 < cols;
            let n = if wide { 2 } else { 1 };
            let changed = (x..x + n).any(|i| self.row[i] != old[i]);
            if changed && !cell.flags.has(Flags::SPACER) {
                self.draw_cell(term, canvas, cell, (x, y, n));
                lo = lo.min(x);
                hi = hi.max(x + n);
            }
            old[x..x + n].copy_from_slice(&self.row[x..x + n]);
            x += n;
        }
        (lo < hi).then(|| self.cell_rect(lo, y, hi - lo))
    }

    fn cell_rect(&self, x: usize, y: usize, n: usize) -> Rect {
        let (cw, ch) = (self.font.cell_w, self.font.cell_h);
        Rect {
            x: self.pad + x * cw,
            y: self.pad + self.top + y * ch,
            w: n * cw,
            h: ch,
        }
    }

    fn draw_cell(
        &mut self,
        term: &Term,
        canvas: &mut Canvas,
        c: Cell,
        (x, y, n): (usize, usize, usize),
    ) {
        let r = self.cell_rect(x, y, n);
        if r.x >= canvas.w || r.y >= canvas.h {
            return; // mid-resize the grid can be larger than the window
        }
        let (mut fg, mut bg) = colors(term, c);
        let f = c.flags.0;
        let block = f & CURSOR != 0 && f & (CURSOR_UNDERLINE | CURSOR_BEAM | CURSOR_HOLLOW) == 0;
        if block {
            (fg, bg) = (color::px(term.palette.bg), term.palette.cursor_px());
        }
        let style =
            usize::from(c.flags.has(Flags::BOLD)) | usize::from(c.flags.has(Flags::ITALIC)) << 1;
        if c.ch == ' ' || c.flags.has(Flags::HIDDEN) {
            canvas.fill(r, bg);
        } else {
            let g = self.cache.get(&mut self.font, c.ch, style, n as u8);
            canvas.glyph(r, &g.data, bg, fg);
        }
        self.decorate(canvas, c, r, fg);
        self.cursor(canvas, f, r, term.palette.cursor_px());
    }

    fn decorate(&self, canvas: &mut Canvas, c: Cell, r: Rect, fg: u32) {
        let t = self.font.thickness;
        let base = r.y + (self.font.baseline + t).min(r.h - t);
        let line = |y: usize| Rect { y, h: t, ..r };
        if c.flags.has(Flags::UNDERLINE) {
            canvas.fill(line(base), fg);
        }
        if c.flags.has(Flags::DOUBLE_UL) {
            canvas.fill(line(base), fg);
            canvas.fill(line((base + 2 * t).min(r.y + r.h - t)), fg);
        }
        if c.flags.has(Flags::CURLY_UL) {
            let h = (r.y + r.h).saturating_sub(base).max(t);
            canvas.curly(Rect { y: base, h, ..r }, self.font.cell_w, t, fg);
        }
        if c.flags.has(Flags::STRIKE) {
            canvas.fill(line(r.y + self.font.baseline * 2 / 3), fg);
        }
    }

    fn cursor(&self, canvas: &mut Canvas, f: u32, r: Rect, color: u32) {
        if f & CURSOR == 0 {
            return;
        }
        let t = self.font.thickness;
        if f & CURSOR_HOLLOW != 0 {
            canvas.frame(
                Rect {
                    w: self.font.cell_w,
                    ..r
                },
                t,
                color,
            );
        } else if f & CURSOR_BEAM != 0 {
            canvas.fill(Rect { w: 2 * t, ..r }, color);
        } else if f & CURSOR_UNDERLINE != 0 {
            let h = 2 * t;
            let y = r.y + r.h - h;
            canvas.fill(
                Rect {
                    y,
                    h,
                    w: self.font.cell_w,
                    ..r
                },
                color,
            );
        }
    }
}

/// Resolved fg/bg of a cell: reverse, then selection (its own background, text
/// keeps its color), then dim.
fn colors(term: &Term, c: Cell) -> (u32, u32) {
    let p = &term.palette;
    let mut fg = p.fg(c.fg, c.flags.has(Flags::BOLD));
    let mut bg = p.bg(c.bg);
    // Text is always opaque: a glass background used as text color (reverse, dim)
    // would let the wallpaper show through the letters.
    let solid = |c: u32| if c >> 24 == 0xff { c } else { color::px(p.bg) };
    if c.flags.has(Flags::REVERSE) {
        (fg, bg) = (solid(bg), fg);
    }
    if c.flags.0 & SELECTED != 0 {
        bg = p.selection_px();
    }
    if c.flags.has(Flags::DIM) {
        fg = draw::blend(solid(bg), fg, 150);
    }
    (fg, bg)
}

/// Where the cursor and selection are on screen this frame.
struct Marks {
    cursor: Option<(usize, usize)>,
    cursor_bits: u32,
    selection: Option<Selection>,
    /// Line number of visible row 0.
    first_line: u64,
}

impl Marks {
    fn new(term: &Term, view: &View) -> Self {
        let c = term.cursor();
        let row = c.row + term.view;
        let visible = term.started && term.modes.show_cursor && row < term.rows();
        let shape = if !view.focused {
            CURSOR_HOLLOW
        } else {
            match term.shape {
                CursorShape::Block => 0,
                CursorShape::Underline => CURSOR_UNDERLINE,
                CursorShape::Beam => CURSOR_BEAM,
            }
        };
        Marks {
            cursor: visible.then_some((row, c.col)),
            cursor_bits: CURSOR | shape,
            selection: view.selection,
            first_line: term.history_pushed() - term.view.min(term.history_len()) as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// Mid-drag the grid lags the window (see app `resize`): it must clip, not panic.
    #[test]
    fn grid_larger_than_canvas_clips() {
        crate::sys::init_locale();
        let cfg = Config::default();
        let mut r = Renderer::new(Font::new(&cfg.font, 11.0).unwrap(), 6, cfg.theme);
        let mut t = Term::new(120, 40, 0, cfg.palette());
        t.feed("wide 漢字 text at the edge of the grid\r\n".repeat(40).as_bytes());
        let (w, h) = (300, 120);
        let mut px = vec![0u32; w * h];
        let view = View { focused: true, selection: None, tabs: &[], active_tab: 0 };
        let damage = r.draw(&t, &view, &mut Canvas { px: &mut px, w, h }, &mut Shown::default());
        assert!(!damage.is_empty());
        assert!(px.iter().any(|&p| p != 0));
    }
}
