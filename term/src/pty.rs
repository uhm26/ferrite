//! The shell's side of the terminal: spawns it on a pty and moves bytes.
//! Reads and writes never block; unwritten input waits in a queue.

use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::sys;

#[derive(Debug)]
pub enum Error {
    Open(io::Error),
    Spawn(String, io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Open(e) => write!(f, "cannot open a pty: {e}"),
            Error::Spawn(cmd, e) => write!(f, "cannot start {cmd}: {e}"),
        }
    }
}

pub struct Pty {
    master: File,
    /// The shell (our child, also across a restart in place).
    pid: u32,
    pending: VecDeque<u8>,
}

/// Result of one read attempt.
pub enum Got {
    Data(usize),
    Empty,
    /// The shell exited (EIO on the master) or the pty closed.
    Closed,
}

pub fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map_or_else(|| "/".into(), Into::into)
}

/// fish if installed, else `$SHELL`, else /bin/sh.
pub fn default_shell() -> String {
    let path = std::env::var_os("PATH").unwrap_or_default();
    if std::env::split_paths(&path).any(|d| d.join("fish").is_file()) {
        return "fish".into();
    }
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

impl Pty {
    /// Start `argv[0]` with the rest as arguments, in `cwd` if given.
    pub fn spawn(
        argv: &[String],
        cwd: Option<&std::path::Path>,
        cols: u16,
        rows: u16,
    ) -> Result<Self, Error> {
        let shell = argv.first().map_or("/bin/sh", String::as_str);
        let (master, slave) = sys::pty::open().map_err(Error::Open)?;
        sys::pty::set_size(master.as_fd(), cols, rows, (0, 0)).map_err(Error::Open)?;
        let io = |fd: &OwnedFd| fd.try_clone().map(Stdio::from).map_err(Error::Open);
        let mut cmd = Command::new(shell);
        cmd.args(argv.iter().skip(1));
        // A gone folder (deleted, or the compositor's stale cwd) would make the shell
        // complain; fall back to $HOME like other terminals.
        let here_ok = std::env::current_dir().is_ok_and(|d| d.is_dir());
        match cwd.filter(|d| d.is_dir()) {
            Some(dir) => {
                cmd.current_dir(dir);
            }
            None if !here_ok => {
                cmd.current_dir(home());
            }
            None => {}
        }
        cmd.stdin(io(&slave)?)
            .stdout(io(&slave)?)
            .stderr(io(&slave)?);
        cmd.env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "term")
            .env_remove("KITTY_WINDOW_ID")
            .env_remove("KITTY_PID")
            .env_remove(crate::app::RESTORE_ENV);
        // SAFETY: the closure only makes async-signal-safe calls (setsid, ioctl).
        unsafe { cmd.pre_exec(sys::pty::become_session_leader) };
        let child = cmd
            .spawn()
            .map_err(|e| Error::Spawn(shell.to_string(), e))?;
        drop(slave);
        sys::set_nonblocking(master.as_fd()).map_err(Error::Open)?;
        Ok(Self {
            master: File::from(master),
            pid: child.id(),
            pending: VecDeque::new(),
        })
    }

    /// Take over a shell handed over by [`Pty::hand_over`] before an exec.
    pub fn adopt(master: OwnedFd, pid: u32) -> io::Result<Self> {
        sys::set_cloexec(master.as_fd(), true)?; // later shells must not inherit it
        sys::set_nonblocking(master.as_fd())?;
        Ok(Self {
            master: File::from(master),
            pid,
            pending: VecDeque::new(),
        })
    }

    /// Keep the shell alive through an exec: the pty stays open across it and the
    /// shell is not killed. Returns the fd number and the shell's pid. Never fails:
    /// if close-on-exec can't be cleared, that one tab is lost at exec (its shell
    /// gets a hangup) instead of the restart aborting with half the tabs handed over.
    pub fn hand_over(self) -> (RawFd, u32) {
        if let Err(e) = sys::set_cloexec(self.master.as_fd(), false) {
            eprintln!("term: a tab won't survive the restart: {e}");
        }
        let this = std::mem::ManuallyDrop::new(self);
        (this.master.as_raw_fd(), this.pid)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.master.as_fd()
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Got {
        match self.master.read(buf) {
            Ok(0) => Got::Closed,
            Ok(n) => Got::Data(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Got::Empty,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Got::Empty,
            Err(_) => Got::Closed, // EIO: the slave side is gone
        }
    }

    /// Queue bytes for the shell and send what the pty takes now.
    pub fn write(&mut self, bytes: &[u8]) {
        self.pending.extend(bytes);
        self.flush();
    }

    pub fn flush(&mut self) {
        while !self.pending.is_empty() {
            let (front, _) = self.pending.as_slices();
            match self.master.write(front) {
                Ok(n) => drop(self.pending.drain(..n)),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return, // WouldBlock (poll for POLLOUT) or gone
            }
        }
    }

    pub fn wants_write(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn resize(&self, cols: u16, rows: u16, px: (u16, u16)) {
        // A failed resize only means the shell keeps its old size.
        let _ = sys::pty::set_size(self.master.as_fd(), cols, rows, px);
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // Kill and reap the shell so no zombie stays (the master closes after this).
        sys::kill_and_reap(self.pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_through_pty() {
        let mut pty = Pty::spawn(&["/bin/sh".into()], None, 80, 24).unwrap();
        pty.write(b"echo hi$((1+1))\nexit\n");
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        for _ in 0..200 {
            let mut fds = [sys::PollFd::new(pty.fd(), sys::POLLIN)];
            sys::wait(&mut fds, 50).unwrap();
            match pty.read(&mut buf) {
                Got::Data(n) => out.extend_from_slice(&buf[..n]),
                Got::Empty => {}
                Got::Closed => break,
            }
        }
        assert!(String::from_utf8_lossy(&out).contains("hi2"));
    }
}
