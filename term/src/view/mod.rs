//! Files in a pane: [`Doc`] opens one, shows it formatted (Preview: [`markdown`]
//! or [`code`], wrapped here so "file line N → screen row" is known for `file:42`)
//! or raw for editing ([`editor`], [`buffer`]), and saves it. Output is VT bytes a
//! `Term` shows; colors are palette indices, so the theme and live reload apply.

mod buffer;
mod code;
mod editor;
mod markdown;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::sys::char_width;
pub use buffer::Buffer;
pub use editor::Outcome;

/// Files bigger than this are cut (a viewer, not a hex dump).
const MAX_BYTES: u64 = 4 << 20;

/// Text style with palette colors (0-255).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Style {
    pub fg: Option<u8>,
    pub bg: Option<u8>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, PartialEq)]
pub struct Span {
    pub style: Style,
    pub text: String,
}

impl Span {
    pub fn new(style: Style, text: impl Into<String>) -> Self {
        Span {
            style,
            text: text.into(),
        }
    }
}

/// One formatted source line.
#[derive(Default, Debug)]
pub struct Line {
    /// 0-based line in the file it came from.
    pub source: usize,
    pub spans: Vec<Span>,
    /// Where wrapped rows continue (under the text, past a gutter or bullet).
    pub indent: usize,
    /// Background to the right edge (code blocks).
    pub fill: Option<u8>,
    /// A full-width horizontal rule instead of text.
    pub rule: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Markdown,
    Code,
}

/// Formatted (read) or raw text with a cursor (edit).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Preview,
    Edit,
}

/// A file opened in a pane.
pub struct Doc {
    pub path: PathBuf,
    pub buf: Buffer,
    kind: Kind,
    pub mode: Mode,
    /// Can be saved: valid UTF-8, not cut, not read-only.
    writable: bool,
    /// The file uses CRLF line ends (kept on save).
    crlf: bool,
    /// What Tab inserts: a tab if the file indents with tabs, else 4 spaces.
    indent: String,
    /// 1-based line to show first (from `file:42`).
    pub line: usize,
    /// Edit mode's first visible line and column.
    top: usize,
    left: usize,
    /// Width of the line-number gutter, as last drawn.
    gutter: usize,
    /// The cursor moved: scroll it into view on the next draw.
    follow: bool,
    /// Closing with unsaved edits was asked once; asking again discards them.
    pub confirm: bool,
    /// A short status for the header ("read-only", a save error, ...).
    pub note: String,
}

impl Doc {
    pub fn open(path: &Path, line: usize) -> std::io::Result<Doc> {
        use std::io::Read;
        let mut bytes = Vec::new();
        let file = std::fs::File::open(path)?;
        let read_only = file.metadata()?.permissions().readonly();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        let cut = bytes.len() as u64 > MAX_BYTES;
        bytes.truncate(MAX_BYTES as usize);
        let (text, note) = match (String::from_utf8(bytes), cut, read_only) {
            (Ok(t), false, false) => (t, ""),
            (Ok(t), false, true) => (t, "read-only"),
            (Ok(t), true, _) => (t, "cut at 4 MiB, read-only"),
            (Err(e), ..) => {
                let note = if cut { "cut at 4 MiB, read-only" } else { "not UTF-8, read-only" };
                (String::from_utf8_lossy(e.as_bytes()).into_owned(), note)
            }
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let kind = if matches!(ext, "md" | "markdown") { Kind::Markdown } else { Kind::Code };
        let writable = note.is_empty();
        // shortcut: mixed line ends all become CRLF on save; keep per-line ends if that bites.
        let crlf = text.contains("\r\n");
        let tabs = text.lines().any(|l| l.starts_with('\t'));
        let mut buf = Buffer::new(&if crlf { text.replace("\r\n", "\n") } else { text });
        buf.go((line.max(1) - 1, 0), false);
        Ok(Doc {
            path: path.to_path_buf(),
            buf,
            kind,
            mode: if writable && kind == Kind::Code { Mode::Edit } else { Mode::Preview },
            writable,
            crlf,
            indent: if tabs { "\t".into() } else { " ".repeat(buffer::TAB) },
            line: line.max(1),
            top: 0,
            left: 0,
            gutter: 0,
            follow: true,
            confirm: false,
            note: note.into(),
        })
    }

    /// The header title: the file name.
    pub fn title(&self) -> String {
        self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into())
    }

    fn ext(&self) -> &str {
        self.path.extension().and_then(|e| e.to_str()).unwrap_or("")
    }

    /// The Preview/Edit switch (Markdown that can be saved): Some(editing).
    pub fn toggle(&self) -> Option<bool> {
        (self.kind == Kind::Markdown && self.writable).then_some(self.mode == Mode::Edit)
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.toggle().is_some() {
            // The preview opens at the line being edited.
            (self.mode, self.line, self.follow) = (mode, self.buf.cur.0 + 1, true);
        }
    }

    /// Write the text back (Ctrl+S): to a temp file next to it, then renamed over
    /// it, so a crash mid-save never leaves half a file. Mode bits are kept.
    pub fn save(&mut self) {
        if !self.writable {
            return;
        }
        match self.write() {
            Ok(()) => {
                self.buf.mark_saved();
                (self.note, self.confirm) = (String::new(), false);
            }
            Err(e) => self.note = format!("not saved: {e}"),
        }
    }

    // shortcut: a rename makes a new inode (hard links split, owner = us); fine for
    // our own files, upgrade to an in-place write if that ever matters.
    fn write(&self) -> std::io::Result<()> {
        use std::io::Write;
        let text = self.buf.text();
        let text = if self.crlf { text.replace('\n', "\r\n") } else { text };
        let target = std::fs::canonicalize(&self.path)?; // a symlink keeps pointing at it
        let perms = std::fs::metadata(&target)?.permissions();
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = target.with_file_name(format!(".{name}.term-save"));
        let res = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.set_permissions(perms)?;
            f.sync_all()?;
            std::fs::rename(&tmp, &target)
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    /// Preview: VT bytes for a pane `cols` wide, and the screen row of `self.line`.
    pub fn render(&self, cols: usize) -> (Vec<u8>, usize) {
        let text = self.buf.text().replace('\t', "    ");
        let lines = match self.kind {
            Kind::Markdown => markdown::format(&text),
            Kind::Code => code::format(&text, self.ext()),
        };
        emit(&lines, cols.max(2), self.line - 1)
    }
}

/// Wrap `lines` to `cols` and write them as VT bytes (cursor hidden). Returns the
/// bytes and the first screen row of source line `target`.
fn emit(lines: &[Line], cols: usize, target: usize) -> (Vec<u8>, usize) {
    let mut out = String::from("\x1b[?25l");
    let (mut row, mut target_row) = (0, None);
    for line in lines {
        if line.source >= target && target_row.is_none() {
            target_row = Some(row);
        }
        if row > 0 {
            out.push_str("\r\n");
        }
        if line.rule {
            sgr(&mut out, Style { fg: Some(8), ..Style::default() });
            out.push_str(&"─".repeat(cols));
            row += 1;
            continue;
        }
        row += wrap(&mut out, line, cols);
    }
    out.push_str("\x1b[0m");
    (out.into_bytes(), target_row.unwrap_or(0))
}

/// One line, wrapped at `cols`; continuation rows start at `line.indent`. Returns
/// the number of rows written.
fn wrap(out: &mut String, line: &Line, cols: usize) -> usize {
    let indent = line.indent.min(cols / 2);
    let (mut col, mut rows) = (0, 1);
    let mut new_row = |out: &mut String, col: &mut usize, style| {
        finish_row(out, line, cols - *col);
        out.push_str("\r\n");
        out.push_str(&" ".repeat(indent));
        sgr(out, style);
        (*col, rows) = (indent, rows + 1);
    };
    for span in &line.spans {
        sgr(out, span.style);
        // Whole words move to the next row; only a word longer than a row is cut.
        for word in span.text.split_inclusive(' ') {
            let width: usize = word.trim_end().chars().map(char_width).sum();
            if col > indent && col + width > cols {
                new_row(out, &mut col, span.style);
            }
            for c in word.chars() {
                let w = char_width(c);
                if col + w > cols {
                    if c == ' ' {
                        continue; // a space at the edge: the wrap is the space
                    }
                    new_row(out, &mut col, span.style);
                }
                out.push(c);
                col += w;
            }
        }
    }
    finish_row(out, line, cols.saturating_sub(col));
    rows
}

/// End a row: reset the style, and paint the fill color to the edge.
fn finish_row(out: &mut String, line: &Line, left: usize) {
    out.push_str("\x1b[0m");
    if let Some(bg) = line.fill {
        let _ = write!(out, "\x1b[48;5;{bg}m{}\x1b[0m", " ".repeat(left));
    }
}

fn sgr(out: &mut String, s: Style) {
    out.push_str("\x1b[0");
    for (on, code) in [(s.bold, ";1"), (s.italic, ";3"), (s.underline, ";4")] {
        if on {
            out.push_str(code);
        }
    }
    if let Some(fg) = s.fg {
        let _ = write!(out, ";38;5;{fg}");
    }
    if let Some(bg) = s.bg {
        let _ = write!(out, ";48;5;{bg}");
    }
    out.push('m');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(spans: &[(&str, Style)], indent: usize) -> Line {
        Line {
            spans: spans.iter().map(|(t, s)| Span::new(*s, *t)).collect(),
            indent,
            ..Line::default()
        }
    }

    /// Feed bytes into a term and read its rows back as text.
    pub(super) fn screen(bytes: &[u8], cols: usize) -> Vec<String> {
        crate::sys::init_locale();
        let mut t = crate::term::Term::new(cols, 30, 100, crate::config::Config::default().palette());
        t.feed(bytes);
        (0..30)
            .map(|y| t.visible_row(y).iter().map(|c| c.ch).collect::<String>().trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    #[test]
    fn wraps_with_indent_and_maps_rows() {
        let lines = [
            Line { source: 0, ..plain(&[("• one two three", Style::default())], 2) },
            Line { source: 1, ..plain(&[("next", Style::default())], 0) },
        ];
        let (bytes, row) = emit(&lines, 8, 1);
        assert_eq!(screen(&bytes, 8), ["• one", "  two", "  three", "next"]);
        assert_eq!(row, 3); // source line 1 starts on screen row 3
    }
}
