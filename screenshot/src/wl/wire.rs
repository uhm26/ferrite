//! The Wayland wire format over the compositor's unix socket: 8-byte header
//! (object id, size << 16 | opcode), then 32-bit args; strings are length-prefixed,
//! NUL-terminated and padded to 4 bytes; fds travel as SCM_RIGHTS ancillary data.
//! Copied from ../clip/src/wl/wire.rs (2026-11-01), which came from ../term and ../wall;
//! dropped: wl_fixed arguments, id reuse; added: i32 event args.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use crate::sys;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    NoDisplay,
    /// The compositor reported a fatal error (wl_display.error).
    Protocol {
        object: u32,
        code: u32,
        message: String,
    },
    Malformed,
    Closed,
    /// A required global is missing (e.g. the data-control manager).
    Missing(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "wayland socket: {e}"),
            Error::NoDisplay => write!(f, "XDG_RUNTIME_DIR is not set; is a compositor running?"),
            Error::Protocol {
                object,
                code,
                message,
            } => write!(
                f,
                "compositor error on object {object} (code {code}): {message}"
            ),
            Error::Malformed => write!(f, "malformed message from the compositor"),
            Error::Closed => write!(f, "compositor closed the connection"),
            Error::Missing(g) => write!(f, "compositor lacks {g}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// One request argument.
pub enum Arg<'a> {
    U(u32),
    I(i32),
    S(&'a str),
    Fd(BorrowedFd<'a>),
}

/// One event, copied out of the read buffer.
pub struct Event {
    pub obj: u32,
    pub op: u16,
    data: Vec<u8>,
}

pub struct Conn {
    sock: UnixStream,
    /// Fds that came with events, in order (keymap, clipboard send).
    fds: VecDeque<OwnedFd>,
    next_id: u32,
    out: Vec<u8>,
    inbuf: Vec<u8>,
    start: usize,
}

fn socket_path() -> Result<PathBuf, Error> {
    let display = std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into());
    let display = PathBuf::from(display);
    if display.is_absolute() {
        return Ok(display);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or(Error::NoDisplay)?;
    Ok(PathBuf::from(dir).join(display))
}

impl Conn {
    pub fn connect() -> Result<Self, Error> {
        let sock = UnixStream::connect(socket_path()?)?;
        Ok(Self {
            sock,
            fds: VecDeque::new(),
            next_id: 2,
            out: Vec::with_capacity(4096),
            inbuf: Vec::new(),
            start: 0,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.sock.as_fd()
    }

    pub fn new_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id - 1
    }

    /// Queue a request. A request carrying an fd is sent at once, so the fd only has
    /// to live for this call.
    pub fn send(&mut self, obj: u32, op: u16, args: &[Arg]) -> Result<(), Error> {
        let at = self.out.len();
        self.out.extend_from_slice(&obj.to_ne_bytes());
        self.out.extend_from_slice(&[0; 4]);
        let mut fd = None;
        for arg in args {
            match arg {
                Arg::U(v) => self.out.extend_from_slice(&v.to_ne_bytes()),
                Arg::I(v) => self.out.extend_from_slice(&v.to_ne_bytes()),
                Arg::S(s) => put_str(&mut self.out, s),
                Arg::Fd(f) => fd = Some(f.as_raw_fd()),
            }
        }
        let size = (self.out.len() - at) as u32;
        let word = (size << 16) | u32::from(op);
        self.out[at + 4..at + 8].copy_from_slice(&word.to_ne_bytes());
        match fd {
            Some(fd) => self.flush_with_fd(fd),
            None => Ok(()),
        }
    }

    pub fn flush(&mut self) -> Result<(), Error> {
        self.sock.write_all(&self.out)?;
        self.out.clear();
        Ok(())
    }

    fn flush_with_fd(&mut self, fd: RawFd) -> Result<(), Error> {
        let n = sys::sock::send_with_fd(self.sock.as_raw_fd(), &self.out, fd)?;
        self.sock.write_all(&self.out[n..])?;
        self.out.clear();
        Ok(())
    }

    /// Read what the socket has, without blocking. Ok(false): nothing there.
    pub fn read(&mut self) -> Result<bool, Error> {
        if self.start > 0 {
            self.inbuf.drain(..self.start);
            self.start = 0;
        }
        let mut chunk = [0u8; 4096];
        let mut fds = Vec::new();
        let n = match sys::sock::recv_with_fds(self.sock.as_raw_fd(), &mut chunk, &mut fds) {
            Ok(0) => return Err(Error::Closed),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        self.fds.extend(fds);
        self.inbuf.extend_from_slice(&chunk[..n]);
        Ok(true)
    }

    /// The next fd that came with the events (events with an fd arg take one each).
    pub fn take_fd(&mut self) -> Result<OwnedFd, Error> {
        self.fds.pop_front().ok_or(Error::Malformed)
    }

    /// The next complete event already read, if any.
    pub fn next_event(&mut self) -> Result<Option<Event>, Error> {
        let buf = &self.inbuf[self.start..];
        if buf.len() < 8 {
            return Ok(None);
        }
        let obj = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let word = u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let size = (word >> 16) as usize;
        if size < 8 || !size.is_multiple_of(4) {
            return Err(Error::Malformed);
        }
        if buf.len() < size {
            return Ok(None);
        }
        let data = buf[8..size].to_vec();
        self.start += size;
        Ok(Some(Event {
            obj,
            op: (word & 0xffff) as u16,
            data,
        }))
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let len = s.len() + 1;
    out.extend_from_slice(&(len as u32).to_ne_bytes());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

impl Event {
    pub fn args(&self) -> Args<'_> {
        Args {
            b: &self.data,
            pos: 0,
        }
    }
}

/// Reads an event's arguments in order.
pub struct Args<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Args<'a> {
    pub fn u32(&mut self) -> Result<u32, Error> {
        let w = self.b.get(self.pos..self.pos + 4).ok_or(Error::Malformed)?;
        self.pos += 4;
        Ok(u32::from_ne_bytes([w[0], w[1], w[2], w[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, Error> {
        self.u32().map(|v| v as i32)
    }

    pub fn array(&mut self) -> Result<&'a [u8], Error> {
        let len = self.u32()? as usize;
        let raw = self
            .b
            .get(self.pos..self.pos + len)
            .ok_or(Error::Malformed)?;
        self.pos += len.div_ceil(4) * 4;
        Ok(raw)
    }

    pub fn str(&mut self) -> Result<&'a str, Error> {
        let raw = self.array()?;
        let text = raw
            .get(..raw.len().saturating_sub(1))
            .ok_or(Error::Malformed)?;
        std::str::from_utf8(text).map_err(|_| Error::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_array_roundtrip() {
        let mut out = Vec::new();
        put_str(&mut out, "wl_shm");
        assert_eq!(out.len(), 4 + 8); // 7 bytes incl. NUL, padded to 8
        out.extend_from_slice(&5u32.to_ne_bytes());
        let ev = Event {
            obj: 2,
            op: 0,
            data: out,
        };
        let mut a = ev.args();
        assert_eq!(a.str().unwrap(), "wl_shm");
        assert_eq!(a.u32().unwrap(), 5);
        assert!(a.u32().is_err());
    }
}
