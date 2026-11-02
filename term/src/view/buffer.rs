//! The editor's text: lines, a cursor and selection, and every change as an edit
//! that undo/redo replays. Positions are (line, byte) on char boundaries; screen
//! columns expand tabs to [`TAB`] and count wide chars twice.

use crate::sys::char_width;

pub const TAB: usize = 4;
/// Undo steps kept (typing a word is one step).
const MAX_UNDO: usize = 1000;

/// (line, byte in that line).
pub type Pos = (usize, usize);

struct Edit {
    id: u64,
    at: Pos,
    removed: String,
    inserted: String,
    /// Cursor before the edit; undo puts it back.
    before: Pos,
}

pub struct Buffer {
    lines: Vec<String>,
    pub cur: Pos,
    /// The other end of the selection, if one is open.
    pub anchor: Option<Pos>,
    /// Screen column up/down aims for, kept across short lines.
    want: Option<usize>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    next_id: u64,
    /// Id of the newest undo step when the file was saved (None: no edits then).
    saved: Option<u64>,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        Buffer {
            lines: text.split('\n').map(String::from).collect(),
            cur: (0, 0),
            anchor: None,
            want: None,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
            saved: None,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    /// Changed since the last save (undoing back to it counts as unchanged).
    pub fn dirty(&self) -> bool {
        self.undo.last().map(|e| e.id) != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.undo.last().map(|e| e.id);
    }

    /// The selection, ordered, if it is not empty.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor.filter(|&a| a != self.cur)?;
        Some((a.min(self.cur), a.max(self.cur)))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.slice(a, b))
    }
}

/// Editing.
impl Buffer {
    /// Replace `from..to` with `text` as one undo step; the cursor ends after it.
    pub fn replace(&mut self, from: Pos, to: Pos, text: &str) {
        let before = self.cur;
        let removed = self.splice(from, to, text);
        self.cur = end_of(from, text);
        (self.anchor, self.want) = (None, None);
        self.redo.clear();
        self.next_id += 1;
        let e = Edit {
            id: self.next_id,
            at: from,
            removed,
            inserted: text.into(),
            before,
        };
        if let Some(top) = self.undo.last_mut()
            && merge(top, &e)
        {
            top.id = e.id;
            return;
        }
        if self.undo.len() == MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(e);
    }

    /// Type `text` over the selection, or at the cursor.
    pub fn insert(&mut self, text: &str) {
        let (a, b) = self.selection().unwrap_or((self.cur, self.cur));
        self.replace(a, b, text);
    }

    /// Enter: a new line indented like this one.
    pub fn newline(&mut self) {
        let line = self.line(self.cur.0);
        let n = line.len() - line.trim_start_matches([' ', '\t']).len();
        let indent = line[..n.min(self.cur.1)].to_string();
        self.insert(&format!("\n{indent}"));
    }

    /// Backspace (`forward`: Delete): the selection, else one char or one word.
    pub fn erase(&mut self, forward: bool, word: bool) {
        if let Some((a, b)) = self.selection() {
            return self.replace(a, b, "");
        }
        let p = self.cur;
        let q = match (forward, word) {
            (false, false) => self.left(p),
            (false, true) => self.word_left(p),
            (true, false) => self.right(p),
            (true, true) => self.word_right(p),
        };
        if q != p {
            self.replace(p.min(q), p.max(q), "");
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.pop() else {
            return false;
        };
        self.splice(e.at, end_of(e.at, &e.inserted), &e.removed);
        (self.cur, self.anchor, self.want) = (e.before, None, None);
        self.redo.push(e);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.redo.pop() else {
            return false;
        };
        self.splice(e.at, end_of(e.at, &e.removed), &e.inserted);
        (self.cur, self.anchor, self.want) = (end_of(e.at, &e.inserted), None, None);
        self.undo.push(e);
        true
    }

    /// The text `from..to`.
    fn slice(&self, from: Pos, to: Pos) -> String {
        let mut s = String::new();
        for l in from.0..=to.0 {
            let line = self.line(l);
            let a = if l == from.0 { from.1 } else { 0 };
            let b = if l == to.0 { to.1 } else { line.len() };
            s.push_str(&line[a..b]);
            if l != to.0 {
                s.push('\n');
            }
        }
        s
    }

    /// Put `text` in place of `from..to` (no undo step). Returns what was there.
    fn splice(&mut self, from: Pos, to: Pos, text: &str) -> String {
        let removed = self.slice(from, to);
        let joined = format!(
            "{}{text}{}",
            &self.lines[from.0][..from.1],
            &self.lines[to.0][to.1..]
        );
        self.lines
            .splice(from.0..=to.0, joined.split('\n').map(String::from));
        removed
    }
}

/// Where `text` ends when it is put at `at`.
fn end_of(at: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => (at.0, at.1 + text.len()),
        Some(i) => (at.0 + text.matches('\n').count(), text.len() - i - 1),
    }
}

/// Fold `e` into `top` if both are one run of typing, or of Backspace/Delete.
/// A space starts a new step, so undo takes back a word at a time.
fn merge(top: &mut Edit, e: &Edit) -> bool {
    let one = |s: &str| s.chars().count() == 1 && s != "\n";
    if top.removed.is_empty() && e.removed.is_empty() && one(&e.inserted) {
        let space_starts = e.inserted == " " && !top.inserted.ends_with(' ');
        if end_of(top.at, &top.inserted) == e.at && !space_starts {
            top.inserted.push_str(&e.inserted);
            return true;
        }
    } else if top.inserted.is_empty() && e.inserted.is_empty() && one(&e.removed) {
        if end_of(e.at, &e.removed) == top.at {
            top.removed.insert_str(0, &e.removed); // Backspace
            top.at = e.at;
            return true;
        }
        if e.at == top.at {
            top.removed.push_str(&e.removed); // Delete
            return true;
        }
    }
    false
}

/// Moving.
impl Buffer {
    /// Put the cursor at `p`; `extend` keeps (or opens) the selection.
    pub fn go(&mut self, p: Pos, extend: bool) {
        self.anchor = if extend {
            self.anchor.or(Some(self.cur))
        } else {
            None
        };
        (self.cur, self.want) = (self.clamp(p), None);
    }

    /// Up or down `n` lines, keeping the column.
    pub fn vertical(&mut self, n: isize, extend: bool) {
        let want = self.want.unwrap_or_else(|| self.col(self.cur));
        let l = self.cur.0.saturating_add_signed(n).min(self.line_count() - 1);
        let to = (l, self.byte_at(l, want));
        self.go(to, extend);
        self.want = Some(want);
    }

    /// Home: the first non-blank char, or the line start if already there.
    pub fn home(&self) -> Pos {
        let line = self.line(self.cur.0);
        let first = line.len() - line.trim_start_matches([' ', '\t']).len();
        (self.cur.0, if self.cur.1 == first { 0 } else { first })
    }

    pub fn end(&self) -> Pos {
        (self.cur.0, self.line(self.cur.0).len())
    }

    pub fn last(&self) -> Pos {
        (self.line_count() - 1, self.line(self.line_count() - 1).len())
    }

    pub fn left(&self, (l, b): Pos) -> Pos {
        match self.line(l)[..b].char_indices().next_back() {
            Some((i, _)) => (l, i),
            None if l > 0 => (l - 1, self.line(l - 1).len()),
            None => (l, b),
        }
    }

    pub fn right(&self, (l, b): Pos) -> Pos {
        match self.line(l)[b..].chars().next() {
            Some(c) => (l, b + c.len_utf8()),
            None if l + 1 < self.line_count() => (l + 1, 0),
            None => (l, b),
        }
    }

    /// Back over blanks, then over one word (or one run of punctuation).
    pub fn word_left(&self, (l, b): Pos) -> Pos {
        if b == 0 {
            return self.left((l, b));
        }
        let s = self.line(l)[..b].trim_end();
        let Some(c) = s.chars().next_back() else {
            return (l, 0);
        };
        let kind = class(c);
        let start = s
            .rfind(|x: char| class(x) != kind)
            .map_or(0, |i| i + s[i..].chars().next().map_or(1, char::len_utf8));
        (l, start)
    }

    /// Over one word (or punctuation run), then over the blanks after it.
    pub fn word_right(&self, (l, b): Pos) -> Pos {
        let line = self.line(l);
        let Some(c) = line[b..].chars().next() else {
            return self.right((l, b));
        };
        let kind = class(c);
        let mut e = line[b..]
            .find(|x: char| class(x) != kind)
            .map_or(line.len(), |i| b + i);
        if kind != Class::Blank {
            e = line[e..]
                .find(|x: char| class(x) != Class::Blank)
                .map_or(line.len(), |i| e + i);
        }
        (l, e)
    }

    /// The word around `p` (double-click), as (start, end).
    pub fn word_at(&self, p: Pos) -> (Pos, Pos) {
        let line = self.line(p.0);
        let Some(c) = line[p.1..].chars().next() else {
            return (p, p);
        };
        let kind = class(c);
        let a = line[..p.1]
            .rfind(|x: char| class(x) != kind)
            .map_or(0, |i| {
                i + line[i..].chars().next().map_or(1, char::len_utf8)
            });
        let b = line[p.1..]
            .find(|x: char| class(x) != kind)
            .map_or(line.len(), |i| p.1 + i);
        ((p.0, a), (p.0, b))
    }

    fn clamp(&self, (l, b): Pos) -> Pos {
        let l = l.min(self.line_count() - 1);
        let line = self.line(l);
        let mut b = b.min(line.len());
        while !line.is_char_boundary(b) {
            b -= 1;
        }
        (l, b)
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Class {
    Word,
    Blank,
    Punct,
}

fn class(c: char) -> Class {
    match c {
        _ if c.is_alphanumeric() || c == '_' => Class::Word,
        _ if c.is_whitespace() => Class::Blank,
        _ => Class::Punct,
    }
}

/// Screen columns.
impl Buffer {
    /// Screen column of `p` (tabs expanded).
    pub fn col(&self, (l, b): Pos) -> usize {
        self.line(l)[..b].chars().fold(0, advance)
    }

    /// The byte in line `l` at screen column `col` (or the line end).
    pub fn byte_at(&self, l: usize, col: usize) -> usize {
        let mut at = 0;
        for (i, c) in self.line(l).char_indices() {
            let next = advance(at, c);
            if next > col {
                return i;
            }
            at = next;
        }
        self.line(l).len()
    }
}

/// The column after `c` when it starts at `col`.
pub fn advance(col: usize, c: char) -> usize {
    if c == '\t' {
        col + TAB - col % TAB
    } else {
        col + char_width(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(b: &mut Buffer, s: &str) {
        s.chars().for_each(|c| b.insert(&c.to_string()));
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut b = Buffer::new("one\ntwo\n");
        assert_eq!(b.line_count(), 3);
        b.go((0, 3), false);
        typed(&mut b, " more words");
        assert_eq!(b.text(), "one more words\ntwo\n");
        assert!(b.dirty());
        b.undo(); // " words"
        assert_eq!(b.text(), "one more\ntwo\n");
        b.undo();
        b.undo();
        assert_eq!(b.text(), "one\ntwo\n");
        assert!(!b.dirty());
        b.redo();
        assert_eq!(b.text(), "one more\ntwo\n");
        b.mark_saved();
        assert!(!b.dirty());
        b.go((1, 3), false);
        b.erase(false, false);
        b.erase(false, false);
        assert_eq!(b.text(), "one more\nt\n");
        b.undo(); // both Backspaces are one step
        assert_eq!((b.text().as_str(), b.dirty()), ("one more\ntwo\n", false));
    }

    #[test]
    fn multi_line_replace_and_newline_indent() {
        let mut b = Buffer::new("    fn a() {\nx\nend");
        b.anchor = Some((0, 8));
        b.cur = (2, 1);
        assert_eq!(b.selected_text().unwrap(), "() {\nx\ne");
        b.insert("");
        assert_eq!(b.text(), "    fn and");
        b.go((0, 6), false);
        b.newline();
        assert_eq!(b.text(), "    fn\n     and");
        assert_eq!(b.cur, (1, 4));
        b.go((0, 0), false);
        b.erase(false, false); // nothing before the start
        b.go(b.last(), false);
        b.erase(true, false); // nothing after the end
        assert_eq!(b.text(), "    fn\n     and");
    }

    #[test]
    fn columns_words_and_lines() {
        let b = Buffer::new("\ta日b\nfoo.bar(baz)  x");
        assert_eq!(b.col((0, 1)), 4);
        assert_eq!(b.col((0, 2 + 3)), 7); // after the wide char
        assert_eq!(b.byte_at(0, 5), 2); // inside 日: before it
        assert_eq!(b.byte_at(0, 99), b.line(0).len());
        assert_eq!(b.word_right((1, 0)), (1, 3));
        assert_eq!(b.word_right((1, 8)), (1, 11));
        assert_eq!(b.word_right((1, 11)), (1, 14)); // ")" then the blanks
        assert_eq!(b.word_left((1, 14)), (1, 11));
        assert_eq!(b.word_left((1, 0)), (0, b.line(0).len()));
        assert_eq!(b.word_at((1, 5)), ((1, 4), (1, 7)));
        let mut b = Buffer::new("long line\nab\nlong line");
        b.go((0, 7), false);
        b.vertical(1, false);
        assert_eq!(b.cur, (1, 2));
        b.vertical(1, true);
        assert_eq!(b.cur, (2, 7)); // the column comes back
        assert_eq!(b.selected_text().unwrap(), "\nlong li");
    }
}
