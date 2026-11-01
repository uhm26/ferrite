//! One tab: a shell on its own pty with its own screen. Background tabs keep
//! reading and parsing output; only the active one is drawn.

use std::path::Path;
use std::time::Instant;

use crate::config::Config;
use crate::pty::{self, Got, Pty};
use crate::term::Term;

pub(super) struct Session {
    pub term: Term,
    pub pty: Pty,
    /// Last title the shell set (OSC 0/2); empty until it sets one.
    pub title: String,
    /// A name the user gave the tab; wins over the shell's title.
    pub name: Option<String>,
    /// Text of a rename in progress (shown in the bar with a caret).
    pub editing: Option<String>,
    /// When the app started a synchronized update (mode 2026), if one is open.
    pub sync_since: Option<Instant>,
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
            pty,
            title: String::new(),
            name: None,
            editing: None,
            sync_since: None,
        })
    }

    /// A tab taken over from before a restart in place.
    pub fn restored(term: Term, pty: Pty, name: Option<String>, title: String) -> Self {
        Session {
            term,
            pty,
            title,
            name,
            editing: None,
            sync_since: None,
        }
    }

    /// Read what the shell wrote, at most `budget` bytes. Returns bytes read and
    /// whether the shell is still there.
    pub fn read(&mut self, buf: &mut [u8], budget: usize) -> (usize, bool) {
        let mut total = 0;
        while total < budget {
            match self.pty.read(buf) {
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
        if !self.term.replies.is_empty() {
            let r = std::mem::take(&mut self.term.replies);
            self.pty.write(&r);
        }
        self.term.bell = false;
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
            self.pty.write(if on { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize, (cw, ch): (usize, usize)) {
        self.term.resize(cols, rows);
        self.term.set_cell_px(cw as u16, ch as u16);
        let px = ((cols * cw) as u16, (rows * ch) as u16);
        self.pty.resize(cols as u16, rows as u16, px);
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
