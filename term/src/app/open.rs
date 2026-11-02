//! Ctrl+click: on selected text, it goes to the split panel typed (not run); on a
//! link, the browser; on a file or folder, its default app (`xdg-open`, the owner's
//! pick), or nano in the split at that line for text with no default app (code
//! files here). Paths may be absolute, `~/…`, or relative to the pane's shell
//! folder; `:line[:col]` (compiler and Claude output) picks the line. Wrapped lines
//! count as one line.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::App;
use super::session::Session;
use crate::render::Selection;
use crate::term::Term;
use crate::term::cell::Flags;

#[derive(Debug, PartialEq)]
pub(super) enum Target {
    Link(String),
    /// An existing file or folder, and the line from a `:line` suffix.
    Path(PathBuf, Option<u32>),
}

impl App {
    /// Ctrl+click on cell (col, row). False if nothing was there to act on.
    pub(super) fn ctrl_click(&mut self, (col, row): (usize, usize)) -> bool {
        let line = self.line_of(row);
        if self
            .mouse
            .selection()
            .is_some_and(|s| s.contains(line, col))
        {
            let term = &self.tabs[self.active].pane().term;
            if let Some(text) = self.mouse.selected_text(term) {
                super::input::log(self.send_to_split(text, false));
                return true;
            }
        }
        self.open_at((col, row))
    }

    /// Open the link or path at cell (col, row). False if there is none.
    fn open_at(&mut self, (col, row): (usize, usize)) -> bool {
        let s = self.tabs[self.active].pane();
        let found = target_under(s, self.line_of(row), col).map(|(t, _)| t);
        let arg = match found {
            Some(Target::Link(url)) => url.into(),
            Some(Target::Path(p, line)) if p.is_file() && !has_default_app(&p) && is_text(&p) => {
                self.open_view(&p, line.unwrap_or(1) as usize);
                return true;
            }
            Some(Target::Path(p, _)) => p.into_os_string(),
            None => return false,
        };
        match Command::new("xdg-open")
            .arg(arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => drop(std::thread::spawn(move || child.wait())), // reap it
            Err(e) => eprintln!("term: xdg-open: {e}"),
        }
        true
    }
}

/// The link or existing path under cell `col` of line `abs` in pane `s`, and where it
/// sits on screen (first and last cell, as stable line numbers) for the hover mark.
pub(super) fn target_under(s: &Session, abs: u64, col: usize) -> Option<(Target, Selection)> {
    let (text, pos, at) = logical_line(&s.term, abs, col)?;
    let (a, b) = token_span(&text, at)?;
    let token: String = text[a..b].iter().collect();
    let found = target(&token, s.cwd().as_deref(), &crate::pty::home())?;
    Some((
        found,
        Selection {
            start: pos[a],
            end: pos[b - 1],
        },
    ))
}

/// A logical line's chars, each char's (line, col), and the clicked char's index.
type LogicalLine = (Vec<char>, Vec<(u64, usize)>, usize);

/// The logical line (wrapped rows joined) around line number `abs`: its chars, each
/// char's (line, col), and the char index of column `col`. Spacer halves of wide
/// chars are left out.
fn logical_line(t: &Term, abs: u64, col: usize) -> Option<LogicalLine> {
    let mut first = abs;
    while first > 0 && t.line_cells(first - 1).is_some_and(|(_, wrapped)| wrapped) {
        first -= 1;
    }
    let (mut text, mut pos, mut at) = (Vec::new(), Vec::new(), None);
    let mut n = first;
    while let Some((cells, wrapped)) = t.line_cells(n) {
        for (x, c) in cells.iter().enumerate() {
            if n == abs && x == col {
                at = Some(text.len());
            }
            if !c.flags.has(Flags::SPACER) {
                text.push(c.ch);
                pos.push((n, x));
            }
        }
        if !wrapped {
            break;
        }
        n += 1;
    }
    Some((text, pos, at?))
}

/// Ends a token: whitespace, and quotes or brackets around paths in prose.
fn is_stop(c: char) -> bool {
    c.is_whitespace() || "\"'`<>()[]{}|│".contains(c)
}

/// The token around char index `at`, as a char range; trailing sentence punctuation
/// is not part of it (so the hover mark stops before a final `.`).
fn token_span(text: &[char], at: usize) -> Option<(usize, usize)> {
    if text.get(at).is_none_or(|&c| is_stop(c)) {
        return None;
    }
    let start = text[..at]
        .iter()
        .rposition(|&c| is_stop(c))
        .map_or(0, |i| i + 1);
    let mut end = text[at..]
        .iter()
        .position(|&c| is_stop(c))
        .map_or(text.len(), |i| at + i);
    while end > start + 1 && ".,;:!?".contains(text[end - 1]) {
        end -= 1;
    }
    (at < end).then_some((start, end))
}

/// What `token` points at: a link by its prefix, a path only if it exists.
fn target(token: &str, cwd: Option<&Path>, home: &Path) -> Option<Target> {
    let t = token.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    if ["https://", "http://", "file://"]
        .iter()
        .any(|p| t.starts_with(p))
    {
        return Some(Target::Link(t.to_string()));
    }
    let (base, line) = strip_line(t);
    let path = match base.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if base.starts_with('/') => PathBuf::from(base),
        None => cwd?.join(base),
    };
    path.exists().then_some(Target::Path(path, line))
}

/// `src/main.rs:42:7` → (`src/main.rs`, 42).
fn strip_line(mut t: &str) -> (&str, Option<u32>) {
    let mut nums = Vec::new();
    for _ in 0..2 {
        match t.rsplit_once(':') {
            Some((head, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                nums.push(n);
                t = head;
            }
            _ => break,
        }
    }
    (t, nums.last().and_then(|n| n.parse().ok())) // file:LINE:col → LINE
}

/// The system has an app for this file type (xdg-mime). Two quick lookups, on a click.
fn has_default_app(p: &Path) -> bool {
    let query = |args: &[&std::ffi::OsStr]| {
        let out = Command::new("xdg-mime")
            .arg("query")
            .args(args)
            .stderr(Stdio::null())
            .output();
        out.ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let Some(mime) = query(&["filetype".as_ref(), p.as_os_str()]).filter(|m| !m.is_empty()) else {
        return false;
    };
    query(&["default".as_ref(), mime.as_ref()]).is_some_and(|app| !app.is_empty())
}

/// Looks like text: no NUL byte in the first 8 KiB.
fn is_text(p: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8192];
    let n = std::fs::File::open(p)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    !head[..n].contains(&0)
}

impl App {
    /// Work out the link or path under the pointer (for the hover mark and the hand
    /// cursor). Only when the pointer moved to another cell: paths cost a stat.
    pub(super) fn update_hover(&mut self) {
        let (x, y) = (self.mouse.x, self.mouse.y);
        let split = self.split_x().is_some_and(|sx| x >= sx as f64);
        let header = if split { self.renderer.bar_height() } else { 0 };
        let over_grid = y >= (self.renderer.top + header) as f64;
        let (col, row) = self.cell_in(split);
        let at = over_grid.then_some((split, col, row));
        if at == self.mouse.hover_at {
            return;
        }
        self.mouse.hover_at = at;
        let s = self.pane_in(split);
        let hover = at
            .and_then(|_| target_under(s, super::mouse::line_in(&s.term, row), col))
            .map(|(_, span)| (split, span));
        if hover != self.mouse.hover {
            self.mouse.hover = hover;
            self.redraw = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str, at: usize) -> Option<String> {
        let chars: Vec<char> = s.chars().collect();
        token_span(&chars, at).map(|(a, b)| chars[a..b].iter().collect())
    }

    #[test]
    fn tokens_stop_at_spaces_quotes_and_brackets() {
        assert_eq!(
            tok("see (src/app/mod.rs:42) now", 8).as_deref(),
            Some("src/app/mod.rs:42")
        );
        assert_eq!(
            tok("open https://example.com/a?b=1.", 10).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(tok("a  b", 1), None);
    }

    #[test]
    fn targets() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let home = Path::new("/nonexistent-home");
        let link = target("https://example.com/x.", None, home);
        assert_eq!(link, Some(Target::Link("https://example.com/x".into())));
        let file = target("src/app/open.rs:12:3,", Some(here), home);
        assert_eq!(
            file,
            Some(Target::Path(here.join("src/app/open.rs"), Some(12)))
        );
        assert_eq!(target("no/such/file.rs", Some(here), home), None);
        assert_eq!(target("relative.rs", None, home), None);
        assert_eq!(
            target("/etc/hostname", None, home),
            Some(Target::Path("/etc/hostname".into(), None))
        );
        assert!(is_text(&here.join("Cargo.toml")));
    }
}
