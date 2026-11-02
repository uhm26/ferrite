//! One pane: a shell on its own pty with its own screen, or a file viewer (no pty;
//! the file is formatted into the screen). Background tabs keep reading and parsing
//! output; only the active one is drawn.

use std::path::Path;
use std::time::Instant;

use crate::config::Config;
use crate::pty::{self, Got, Pty};
use crate::term::Term;
use crate::view::Doc;

pub(super) struct Session {
    pub term: Term,
    /// The shell; None for a file viewer.
    pub pty: Option<Pty>,
    /// The file a viewer shows.
    pub doc: Option<Doc>,
    /// Last title the shell set (OSC 0/2); empty until it sets one.
    pub title: String,
    /// A name the user gave the tab; wins over the shell's title.
    pub name: Option<String>,
    /// Text of a rename in progress (shown in the bar with a caret).
    pub editing: Option<String>,
    /// When the app started a synchronized update (mode 2026), if one is open.
    pub sync_since: Option<Instant>,
    /// The split panel next to this tab (its own shell), if open.
    pub split: Option<Box<Session>>,
    /// Keys and mouse go to the split panel, not the main pane.
    pub focus_split: bool,
    /// Text to type into this shell once it has printed something (prompt ready;
    /// typed earlier it would mix with fish's startup queries), and whether to press
    /// Enter after it.
    pub pending: Option<(String, bool)>,
}

impl Session {
    pub fn spawn(
        cfg: &Config,
        argv: &[String],
        cwd: Option<&Path>,
        (cols, rows): (usize, usize),
    ) -> Result<Self, pty::Error> {
        let pty = Pty::spawn(argv, cwd, cols as u16, rows as u16)?;
        Ok(Session {
            term: Term::new(cols, rows, cfg.scrollback, cfg.palette()),
            pty: Some(pty),
            doc: None,
            title: String::new(),
            name: None,
            editing: None,
            sync_since: None,
            split: None,
            focus_split: false,
            pending: None,
        })
    }

    /// A tab taken over from before a restart in place.
    pub fn restored(term: Term, pty: Pty, name: Option<String>, title: String) -> Self {
        Session {
            term,
            pty: Some(pty),
            doc: None,
            title,
            name,
            editing: None,
            sync_since: None,
            split: None,
            focus_split: false,
            pending: None,
        }
    }

    /// Read what the shell wrote, at most `budget` bytes. Returns bytes read and
    /// whether the shell is still there.
    pub fn read(&mut self, buf: &mut [u8], budget: usize) -> (usize, bool) {
        let mut total = 0;
        while total < budget {
            let Some(pty) = self.pty.as_mut() else {
                break; // a viewer: nothing to read
            };
            match pty.read(buf) {
                Got::Data(n) => {
                    self.term.feed(&buf[..n]);
                    total += n;
                }
                Got::Empty => break,
                Got::Closed => return (total, false),
            }
        }
        (total, true)
    }

    /// Hand back replies, pick up a new title, track synchronized updates.
    /// Returns whether the title changed.
    pub fn settle(&mut self) -> bool {
        if self.term.started
            && let Some((text, enter)) = self.pending.take()
        {
            self.type_in(&text, enter);
        }
        if !self.term.replies.is_empty() {
            let r = std::mem::take(&mut self.term.replies);
            self.write(&r);
        }
        match (self.term.modes.sync, self.sync_since) {
            (true, None) => self.sync_since = Some(Instant::now()),
            (false, Some(_)) => self.sync_since = None,
            _ => {}
        }
        match self.term.title.take() {
            Some(t) if t != self.title => {
                self.title = t;
                true
            }
            _ => false,
        }
    }

    /// Tell the app about focus, if it asked (mode 1004).
    pub fn focus(&mut self, on: bool) {
        if self.term.modes.focus_events {
            self.write(if on { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize, (cw, ch): (usize, usize)) {
        if self.doc.is_some() {
            return self.show(cols, rows); // re-wrapped for the new width
        }
        self.term.resize(cols, rows);
        self.term.set_cell_px(cw as u16, ch as u16);
        let px = ((cols * cw) as u16, (rows * ch) as u16);
        if let Some(pty) = &self.pty {
            pty.resize(cols as u16, rows as u16, px);
        }
    }

    /// The label in the tab bar: a rename in progress, the user's name, the shell's
    /// title, or "shell".
    pub fn label(&self) -> String {
        match &self.editing {
            Some(text) => format!("{text}▏"),
            None => self.title(),
        }
    }

    /// The window title while this tab is active.
    pub fn title(&self) -> String {
        match (&self.name, self.title.is_empty()) {
            (Some(n), _) => n.clone(),
            (None, true) => "shell".into(),
            (None, false) => self.title.clone(),
        }
    }
}

impl Session {
    /// A file viewer pane: `doc` formatted into the screen, no shell.
    pub fn viewer(cfg: &Config, doc: Doc, (cols, rows): (usize, usize)) -> Self {
        let mut s = Session {
            term: Term::new(cols, rows, 0, cfg.palette()),
            pty: None,
            doc: Some(doc),
            title: String::new(),
            name: None,
            editing: None,
            sync_since: None,
            split: None,
            focus_split: false,
            pending: None,
        };
        s.show(cols, rows);
        s
    }

    /// (Re)draw the file at this size. Preview: every row lives in the term
    /// (history + screen), scrolled so the target line sits near the top, so the
    /// wheel, selection and copy work as in a shell. Edit: just the visible lines.
    fn show(&mut self, cols: usize, rows: usize) {
        let Some(doc) = self.doc.as_mut() else { return };
        self.title = doc.title();
        if doc.mode == crate::view::Mode::Edit {
            if (self.term.cols(), self.term.rows(), self.term.history_len()) != (cols, rows, 0) {
                self.term = Term::new(cols, rows, 0, self.term.palette.clone());
            }
            let sel = self.term.palette.selection;
            let bytes = doc.render_edit(cols, rows, sel);
            return self.term.feed(&bytes);
        }
        let (bytes, target) = doc.render(cols);
        let lines = bytes.iter().filter(|&&b| b == b'\n').count() + 1;
        let mut term = Term::new(cols, rows, lines, self.term.palette.clone());
        term.feed(&bytes);
        let top = target.saturating_sub(2).min(term.history_len()); // 2 rows of context
        term.view = term.history_len() - top;
        self.term = term;
    }

    /// Redraw a file pane after a change, at its size.
    pub fn refresh(&mut self) {
        let (cols, rows) = (self.term.cols(), self.term.rows());
        self.show(cols, rows);
    }

    /// Send bytes to the shell (a viewer has none: dropped).
    pub fn write(&mut self, bytes: &[u8]) {
        if let Some(pty) = self.pty.as_mut() {
            pty.write(bytes);
        }
    }

    /// The folder relative paths are taken from: the shell's, or the viewed file's.
    pub fn cwd(&self) -> Option<std::path::PathBuf> {
        match (&self.pty, &self.doc) {
            (Some(p), _) => p.cwd(),
            (None, Some(d)) => d.path.parent().map(Path::to_path_buf),
            (None, None) => None,
        }
    }
}

impl Session {
    /// The pane that has focus: the split panel or this main pane.
    pub fn pane(&self) -> &Session {
        match &self.split {
            Some(s) if self.focus_split => s,
            _ => self,
        }
    }

    pub fn pane_mut(&mut self) -> &mut Session {
        // Checked first, borrowed after: returning a borrow from inside the condition
        // is something the borrow checker can't follow yet.
        if self.focus_split && self.split.is_some() {
            return self.split.as_deref_mut().unwrap_or_else(|| unreachable!());
        }
        self
    }
}

impl Session {
    /// Type `text` at the prompt as a paste (so a multi-line command shows up whole
    /// instead of running line by line), then Enter if `enter`.
    pub fn type_in(&mut self, text: &str, enter: bool) {
        let clean = text.replace('\x1b', "");
        if self.term.modes.bracketed_paste {
            self.write(b"\x1b[200~");
            self.write(clean.as_bytes());
            self.write(b"\x1b[201~");
        } else {
            self.write(clean.replace('\n', " ").as_bytes());
        }
        if enter {
            self.write(b"\r");
        }
    }

    /// Flush pending input and read output if `fd` (this pty's poll slot) says so.
    /// Returns bytes read and whether the shell is still there.
    pub fn pump(&mut self, fd: crate::sys::PollFd, buf: &mut [u8], budget: usize) -> (usize, bool) {
        if fd.ready(crate::sys::POLLOUT)
            && let Some(pty) = self.pty.as_mut()
        {
            pty.flush();
        }
        if fd.ready(super::READY) {
            return self.read(buf, budget);
        }
        (0, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text queued before the shell is ready is typed once it prints its prompt.
    #[test]
    fn pending_text_is_typed_when_the_prompt_shows() {
        crate::sys::init_locale();
        let cfg = Config::default();
        let mut s = Session::spawn(&cfg, &["/bin/sh".into()], None, (40, 5)).unwrap();
        s.pending = Some(("echo ready$((40+2))".into(), true));
        let mut buf = vec![0u8; 4096];
        let mut seen = String::new();
        for _ in 0..200 {
            let mut fds = [crate::sys::PollFd::new(
                s.pty.as_ref().unwrap().fd(),
                crate::sys::POLLIN,
            )];
            crate::sys::wait(&mut fds, 20).unwrap();
            s.read(&mut buf, 1 << 16);
            s.settle();
            seen = (0..5)
                .map(|y| {
                    s.term
                        .visible_row(y)
                        .iter()
                        .map(|c| c.ch)
                        .collect::<String>()
                })
                .collect();
            if seen.contains("ready42") {
                break;
            }
        }
        assert!(seen.contains("ready42"), "{seen}");
        assert!(s.pending.is_none());
    }
}
