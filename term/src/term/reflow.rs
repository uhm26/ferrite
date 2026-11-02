//! Reflow: on resize, the primary screen and its scrollback are rewrapped to the new
//! width, like kitty and foot do. Rows joined by automatic wraps form one logical
//! line; each line is cut again at the new width, so narrowing never loses text and
//! widening joins it back. The cursor keeps its place in its line. Lines that need no
//! rewrap are moved, not copied, so a big scrollback reflows cheaply.

// shortcut: scrollback is capped in rows, so narrowing (long lines take more rows) can
// push the oldest lines out; count the cap in logical lines if that bites.
use std::collections::VecDeque;

use super::Cursor;
use super::cell::{Cell, Flags};
use super::grid::{Grid, History, Row};

pub(super) fn reflow(
    grid: &mut Grid,
    history: &mut History,
    cur: &mut Cursor,
    (cols, rows): (usize, usize),
) {
    let mut all = history.take();
        let cursor_at = all.len() + cur.row;
        let mut screen = grid.take();
        // Blank rows below the cursor are not content; drop them.
        let last = screen
            .iter()
            .rposition(|r| r.cells.iter().any(|c| *c != Cell::BLANK));
        screen.truncate(last.unwrap_or(0).max(cur.row) + 1);
        all.extend(screen);
        let (lines, at) = rewrap(all, cols, (cursor_at, cur.col));
    place(grid, history, cur, (lines, at), (cols, rows));
}

/// All rows cut to `cols`; returns them and the cursor's new (row, col) among them.
fn rewrap(
    old: VecDeque<Row>,
    cols: usize,
    (cursor_row, cursor_col): (usize, usize),
) -> (Vec<Row>, (usize, usize)) {
    let mut out = Vec::with_capacity(old.len());
    let mut at = (0, 0);
    let mut group: Vec<Row> = Vec::new();
    let mut group_start = 0;
    let total = old.len();
    for (i, row) in old.into_iter().enumerate() {
        let wrapped = row.wrapped && i + 1 < total;
        group.push(row);
        if wrapped {
            continue;
        }
        let cursor = (group_start..=i).contains(&cursor_row).then(|| {
            // Offset of the cursor in the joined line.
            group[..cursor_row - group_start]
                .iter()
                .map(|r| r.cells.len())
                .sum::<usize>()
                + cursor_col
        });
        let first = out.len();
        let pos = line(&mut group, cols, cursor, &mut out); // empties `group`, keeps its memory
        if let Some(p) = pos {
            at = (first + p.0, p.1);
        }
        group_start = i + 1;
    }
    (out, at)
}

/// Rewrap one logical line into `out`. Returns the cursor's (row, col) relative to
/// the first row of this line, if the cursor is in it.
fn line(
    rows: &mut Vec<Row>,
    cols: usize,
    cursor: Option<usize>,
    out: &mut Vec<Row>,
) -> Option<(usize, usize)> {
    let fits = rows.len() == 1 && trimmed(&rows[0].cells) <= cols;
    if fits && cursor.is_none_or(|c| c < cols) {
        let mut row = rows.pop()?;
        row.wrapped = false;
        out.push(row);
        return cursor.map(|c| (0, c));
    }
    let mut cells: Vec<Cell> = rows.drain(..).flat_map(|r| r.cells).collect();
    cells.truncate(trimmed(&cells));
    if let Some(c) = cursor {
        cells.resize(cells.len().max(c + 1), Cell::BLANK); // the cursor's cell exists
    }
    let mut start = 0;
    let mut pos = None;
    let mut n = 0;
    while start < cells.len() || n == 0 {
        let mut end = (start + cols).min(cells.len());
        // A wide char never splits: move it (with its spacer) to the next row.
        if end < cells.len() && end > start + 1 && cells[end - 1].flags.has(Flags::WIDE) {
            end -= 1;
        }
        if let Some(c) = cursor.filter(|&c| c >= start && c < end) {
            pos = Some((n, c - start));
        }
        out.push(Row {
            cells: cells[start..end].to_vec(),
            wrapped: end < cells.len(),
        });
        start = end;
        n += 1;
    }
    pos
}

fn trimmed(cells: &[Cell]) -> usize {
    cells
        .iter()
        .rposition(|c| *c != Cell::BLANK)
        .map_or(0, |i| i + 1)
}

/// Put the last `rows` rows on screen (keeping the cursor visible), the rest in
/// history, and move the cursor there.
fn place(
    grid: &mut Grid,
    history: &mut History,
    cur: &mut Cursor,
    (lines, at): (Vec<Row>, (usize, usize)),
    (cols, rows): (usize, usize),
) {
    let start = lines.len().saturating_sub(rows).min(at.0);
    let mut screen = VecDeque::with_capacity(rows);
    for (i, mut row) in lines.into_iter().enumerate() {
        if i < start {
            history.push_row(row);
        } else if screen.len() < rows {
            row.cells.resize(cols, Cell::BLANK);
            screen.push_back(row);
        }
    }
    while screen.len() < rows {
        screen.push_back(Row::new(cols, Cell::BLANK));
    }
    grid.put(screen, cols);
    cur.row = at.0 - start;
    cur.col = at.1.min(cols - 1);
    cur.pending_wrap = false;
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::term::Term;

    fn term(cols: usize, rows: usize) -> Term {
        crate::sys::init_locale();
        Term::new(cols, rows, 100, Config::default().palette())
    }

    fn text(t: &Term) -> Vec<String> {
        let first = t.history_pushed() - t.history_len() as u64;
        (first..first + (t.history_len() + t.rows()) as u64)
            .filter_map(|n| t.line_cells(n))
            .map(|(c, _)| {
                c.iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn narrow_then_wide_keeps_text() {
        let mut t = term(10, 4);
        t.feed(b"0123456789abc\r\n$ ");
        t.resize(5, 4);
        assert_eq!(text(&t), ["01234", "56789", "abc", "$"]);
        assert_eq!((t.cursor().row, t.cursor().col), (3, 2));
        t.resize(20, 4);
        assert_eq!(text(&t)[..2], ["0123456789abc", "$"]);
        assert_eq!((t.cursor().row, t.cursor().col), (1, 2));
    }

    #[test]
    fn wide_char_moves_whole() {
        let mut t = term(6, 3);
        t.feed("ab漢字".as_bytes());
        t.resize(3, 3);
        assert_eq!(text(&t)[..3], ["ab", "漢", "字"]);
    }

    #[test]
    fn rows_only_keeps_cursor_line() {
        let mut t = term(5, 4);
        t.feed(b"a\r\nb\r\nc\r\nd");
        t.resize(5, 2);
        assert_eq!(t.history_len(), 2);
        assert_eq!((t.cursor().row, t.cursor().col), (1, 1));
        t.resize(5, 6);
        assert_eq!(t.history_len(), 0);
        assert_eq!(t.cursor().row, 3);
    }
}
