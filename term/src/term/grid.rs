//! Rows of cells. The screen is a ring of rows, so scrolling the whole screen moves
//! no cells: the top row goes to history, a recycled row comes in at the bottom.
//! History rows are trimmed of trailing blanks to keep scrollback small.

use std::collections::VecDeque;

use super::cell::Cell;

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The line continues on the next row (automatic wrap).
    pub wrapped: bool,
}

impl Row {
    pub fn new(cols: usize, blank: Cell) -> Self {
        Row {
            cells: vec![blank; cols],
            wrapped: false,
        }
    }

    /// Reuse this row's memory as a blank row of `cols` cells.
    fn reset(&mut self, cols: usize, blank: Cell) {
        self.cells.clear();
        self.cells.resize(cols, blank);
        self.wrapped = false;
    }

    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::BLANK);
        if let Some(last) = self.cells.last_mut()
            && last.flags.has(super::cell::Flags::WIDE)
        {
            *last = Cell::BLANK; // its second half was cut off
        }
    }
}

/// Lines that scrolled off the top of the primary screen. Oldest first.
pub struct History {
    rows: VecDeque<Row>,
    cap: usize,
    /// Rows ever pushed; lets callers keep stable line numbers.
    pub pushed: u64,
}

impl History {
    pub fn new(cap: usize) -> Self {
        History {
            rows: VecDeque::new(),
            cap,
            pushed: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&Row> {
        self.rows.get(i)
    }

    /// Store a trimmed copy of `row` (one exact-size allocation); the caller keeps
    /// the full-width original for reuse, so screen rows never reallocate.
    pub fn push(&mut self, row: &Row) {
        self.pushed += 1;
        if self.cap == 0 {
            return;
        }
        // Wrapped rows stay whole: reflow joins them back exactly.
        let keep = if row.wrapped { row.cells.len() } else { trimmed_len(&row.cells) };
        if self.rows.len() == self.cap {
            self.rows.pop_front();
        }
        self.rows.push_back(Row {
            cells: row.cells[..keep].to_vec(),
            wrapped: row.wrapped,
        });
    }

    /// Store a row we already own (reflow output), trimmed like `push`.
    pub fn push_row(&mut self, mut row: Row) {
        if !row.wrapped {
            row.cells.truncate(trimmed_len(&row.cells));
        }
        self.pushed += 1;
        if self.cap == 0 {
            return;
        }
        if self.rows.len() == self.cap {
            self.rows.pop_front();
        }
        self.rows.push_back(row);
    }

    /// Take every row out (for reflow); line numbers continue where they were.
    pub fn take(&mut self) -> VecDeque<Row> {
        self.pushed -= self.rows.len() as u64;
        std::mem::take(&mut self.rows)
    }

    pub fn clear(&mut self) {
        self.rows = VecDeque::new();
    }
}

/// Length without trailing blank cells. Checks 8 cells at a time without early
/// exit inside a chunk, so the compiler can use SIMD compares.
fn trimmed_len(cells: &[Cell]) -> usize {
    let mut end = cells.len();
    while end >= 8 && cells[end - 8..end].iter().fold(true, |all, c| all & (*c == Cell::BLANK)) {
        end -= 8;
    }
    while end > 0 && cells[end - 1] == Cell::BLANK {
        end -= 1;
    }
    end
}

pub struct Grid {
    rows: VecDeque<Row>,
    cols: usize,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Grid {
            rows: (0..rows).map(|_| Row::new(cols, Cell::BLANK)).collect(),
            cols,
        }
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn row(&self, y: usize) -> &Row {
        &self.rows[y]
    }

    pub fn row_mut(&mut self, y: usize) -> &mut Row {
        &mut self.rows[y]
    }

    /// Scroll rows `top..=bot` up by `n`; new rows at the bottom are `blank`.
    /// With `history`, the rows leaving the top are saved there.
    pub fn scroll_up(
        &mut self,
        (top, bot): (usize, usize),
        n: usize,
        blank: Cell,
        mut history: Option<&mut History>,
    ) {
        let n = n.min(bot + 1 - top);
        for _ in 0..n {
            let Some(mut row) = self.rows.remove(top) else {
                return;
            };
            if let Some(h) = history.as_deref_mut() {
                h.push(&row);
            }
            row.reset(self.cols, blank);
            self.rows.insert(bot, row);
        }
    }

    /// Scroll rows `top..=bot` down by `n`; new rows at the top are `blank`.
    pub fn scroll_down(&mut self, (top, bot): (usize, usize), n: usize, blank: Cell) {
        let n = n.min(bot + 1 - top);
        for _ in 0..n {
            let Some(mut row) = self.rows.remove(bot) else {
                return;
            };
            row.reset(self.cols, blank);
            self.rows.insert(top, row);
        }
    }

    pub fn set_cols(&mut self, cols: usize) {
        self.cols = cols;
        for row in &mut self.rows {
            row.resize(cols);
        }
    }

    /// Take every row out (for reflow).
    pub fn take(&mut self) -> VecDeque<Row> {
        std::mem::take(&mut self.rows)
    }

    /// New rows, each exactly `cols` cells.
    pub fn put(&mut self, rows: VecDeque<Row>, cols: usize) {
        self.rows = rows;
        self.cols = cols;
    }

    pub fn truncate(&mut self, rows: usize) {
        self.rows.truncate(rows);
    }

    pub fn grow(&mut self, rows: usize) {
        while self.rows.len() < rows {
            self.rows.push_back(Row::new(self.cols, Cell::BLANK));
        }
    }

    pub fn clear(&mut self, blank: Cell) {
        for row in &mut self.rows {
            row.reset(self.cols, blank);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(c: char) -> Cell {
        Cell {
            ch: c,
            ..Cell::BLANK
        }
    }

    #[test]
    fn full_scroll_feeds_history_trimmed() {
        let mut g = Grid::new(4, 2);
        g.row_mut(0).cells[0] = letter('a');
        let mut h = History::new(10);
        g.scroll_up((0, 1), 1, Cell::BLANK, Some(&mut h));
        assert_eq!(h.len(), 1);
        assert_eq!(h.get(0).unwrap().cells, vec![letter('a')]);
        assert_eq!(g.rows(), 2);
        assert_eq!(g.row(1).cells.len(), 4);
    }

    #[test]
    fn history_cap_recycles() {
        let mut h = History::new(2);
        for _ in 0..3 {
            h.push(&Row::new(3, letter('x')));
        }
        assert_eq!(h.len(), 2);
        assert_eq!(h.pushed, 3);
    }

    #[test]
    fn region_scroll_down() {
        let mut g = Grid::new(2, 3);
        g.row_mut(0).cells[0] = letter('a');
        g.scroll_down((0, 2), 1, Cell::BLANK);
        assert_eq!(g.row(1).cells[0], letter('a'));
        assert_eq!(g.row(0).cells[0], Cell::BLANK);
    }
}
