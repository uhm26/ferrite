//! Restart in place (Ctrl+Shift+R): run the freshly installed term without closing
//! any shell. Each tab's pty stays open across exec() and its shell stays our child
//! (exec keeps the pid). The tabs' state (screen and scrollback as VT bytes, cursor,
//! modes, name, title) travels in an inherited memfd whose number is in
//! `TERM_RESTORE`. Only the window is recreated.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use super::session::Session;
use super::{App, Error};
use crate::config::Config;
use crate::pty::Pty;
use crate::sys;
use crate::term::Term;

/// Names the inherited state fd. Shells never see it (removed in `Pty::spawn`).
pub const RESTORE_ENV: &str = "TERM_RESTORE";
const MAGIC: &[u8] = b"termrst1";

pub(super) struct SavedTab {
    fd: RawFd,
    pid: u32,
    size: (u16, u16),
    name: Option<String>,
    title: String,
    screen: Vec<u8>,
}

pub(super) struct Saved {
    pub active: usize,
    pub font_size: f32,
    tabs: Vec<SavedTab>,
}

impl App {
    /// Exec the installed binary with our tabs. Only returns if that failed; the
    /// tabs are then taken back as they were.
    pub(super) fn restart(&mut self) -> Result<(), Error> {
        if self.keep_unsaved(None) {
            return Ok(()); // files are not carried over: their edits would be lost
        }
        self.finish_rename()?;
        let mut saved = Saved {
            active: self.active,
            font_size: self.renderer.font.size,
            tabs: Vec::new(),
        };
        for s in std::mem::take(&mut self.tabs) {
            let screen = s.term.dump();
            let size = (s.term.cols() as u16, s.term.rows() as u16);
            let (name, title) = (s.name, s.title);
            let Some(pty) = s.pty else {
                continue; // only shells survive a restart; viewers are reopened by hand
            };
            let (fd, pid) = pty.hand_over();
            let tab = SavedTab {
                fd,
                pid,
                size,
                name,
                title,
                screen,
            };
            saved.tabs.push(tab);
        }
        let file = File::from(sys::mem::inheritable_file()?);
        (&file).write_all(&saved.encode())?;
        let err = std::process::Command::new(installed_exe())
            .env(RESTORE_ENV, file.as_raw_fd().to_string())
            .exec();
        eprintln!("term: restart failed: {err}");
        self.tabs = sessions(&self.cfg, saved);
        if self.tabs.is_empty() {
            self.first_tab(Vec::new())?; // never run on with no tab (index panics)
        }
        self.active = self.active.min(self.tabs.len() - 1);
        self.refresh_labels();
        Ok(())
    }
}

/// Our binary's path. After `install` replaced it, /proc/self/exe reads
/// "<path> (deleted)": the new file is at the same path.
fn installed_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| "term".into());
    let s = exe.to_string_lossy();
    match s.strip_suffix(" (deleted)") {
        Some(p) => PathBuf::from(p),
        None => exe,
    }
}

/// The state a restart left us, if we are the restarted program.
pub(super) fn take_saved() -> Option<Saved> {
    let raw: RawFd = std::env::var(RESTORE_ENV).ok()?.parse().ok()?;
    let mut file = File::from(sys::adopt_fd(raw)?);
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_end(&mut bytes).ok()?;
    Saved::decode(&bytes)
}

/// Rebuild tabs: adopt each pty and replay its screen into a fresh terminal.
/// A tab whose pty is gone is dropped.
pub(super) fn sessions(cfg: &Config, saved: Saved) -> Vec<Session> {
    let mut out = Vec::new();
    for t in saved.tabs {
        let Some(pty) = sys::adopt_fd(t.fd).and_then(|fd| Pty::adopt(fd, t.pid).ok()) else {
            continue;
        };
        let (cols, rows) = (usize::from(t.size.0), usize::from(t.size.1));
        let mut term = Term::new(cols, rows, cfg.scrollback, cfg.palette());
        term.feed(&t.screen);
        term.replies.clear();
        out.push(Session::restored(term, pty, t.name, t.title));
    }
    out
}

impl Saved {
    fn encode(&self) -> Vec<u8> {
        let mut w = MAGIC.to_vec();
        put(&mut w, &(self.active as u32).to_le_bytes());
        put(&mut w, &self.font_size.to_le_bytes());
        for t in &self.tabs {
            put(&mut w, &t.fd.to_le_bytes());
            put(&mut w, &t.pid.to_le_bytes());
            put(&mut w, &t.size.0.to_le_bytes());
            put(&mut w, &t.size.1.to_le_bytes());
            put(&mut w, t.name.as_deref().unwrap_or("\0").as_bytes());
            put(&mut w, t.title.as_bytes());
            put(&mut w, &t.screen);
        }
        w
    }

    fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader(b.strip_prefix(MAGIC)?);
        let active = u32::from_le_bytes(r.array()?) as usize;
        let font_size = f32::from_le_bytes(r.array()?);
        let mut tabs = Vec::new();
        while !r.0.is_empty() {
            let fd = RawFd::from_le_bytes(r.array()?);
            let pid = u32::from_le_bytes(r.array()?);
            let size = (
                u16::from_le_bytes(r.array()?),
                u16::from_le_bytes(r.array()?),
            );
            let name = String::from_utf8(r.field()?.to_vec()).ok()?;
            let name = (name != "\0").then_some(name);
            let title = String::from_utf8(r.field()?.to_vec()).ok()?;
            let screen = r.field()?.to_vec();
            tabs.push(SavedTab {
                fd,
                pid,
                size,
                name,
                title,
                screen,
            });
        }
        Some(Saved {
            active,
            font_size,
            tabs,
        })
    }
}

/// Length-prefixed field.
fn put(w: &mut Vec<u8>, bytes: &[u8]) {
    w.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    w.extend_from_slice(bytes);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn field(&mut self) -> Option<&'a [u8]> {
        let len = u32::from_le_bytes(self.0.get(..4)?.try_into().ok()?) as usize;
        let field = self.0.get(4..4 + len)?;
        self.0 = &self.0[4 + len..];
        Some(field)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.field()?.try_into().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip() {
        let tab = |fd, name: Option<&str>| SavedTab {
            fd,
            pid: 4242,
            size: (100, 30),
            name: name.map(String::from),
            title: "~/projects".into(),
            screen: b"\x1b[1mhi\x1b[0m".to_vec(),
        };
        let saved = Saved {
            active: 1,
            font_size: 12.5,
            tabs: vec![tab(7, None), tab(9, Some("build"))],
        };
        let back = Saved::decode(&saved.encode()).unwrap();
        assert_eq!((back.active, back.font_size), (1, 12.5));
        assert_eq!(back.tabs.len(), 2);
        let t = &back.tabs[1];
        assert_eq!((t.fd, t.pid, t.size), (9, 4242, (100, 30)));
        assert_eq!(t.name.as_deref(), Some("build"));
        assert_eq!(back.tabs[0].name, None);
        assert_eq!(t.screen, b"\x1b[1mhi\x1b[0m");
        assert!(Saved::decode(b"garbage").is_none());
    }
}
