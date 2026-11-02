//! Non-blocking transfers, so a slow or stuck app never stalls the daemon: a
//! `Reader` splices a new copy into a memfd, a `Writer` splices it to a paste.
//! The bytes never pass through our heap.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::FileTypeExt;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::sys::{self, splice};

/// Bigger copies are dropped rather than kept in memory.
const MAX_BYTES: i64 = 64 << 20;
/// Bytes moved per syscall; the kernel moves less if less is there.
const CHUNK: i64 = 1 << 20;
/// A copy that sends nothing for this long is given up on (the app hung or
/// leaked the pipe), so it can't block keeping the rest of the copy.
pub const STALL: Duration = Duration::from_secs(10);

/// One kept copy: a memfd nobody writes to again, and its length.
pub struct Kept {
    fd: OwnedFd,
    len: i64,
}

/// Pulls one copy out of the app that made it.
pub struct Reader {
    pipe: OwnedFd,
    kept: Kept,
    /// When data last came in.
    last: Instant,
}

impl Reader {
    pub fn new(pipe: OwnedFd) -> io::Result<Self> {
        sys::set_nonblocking(pipe.as_fd())?; // our own pipe, nobody else shares it
        let _ = splice::grow_pipe(pipe.as_fd()); // only a speedup; 64 KiB works too
        let kept = Kept {
            fd: splice::memfd(c"clipboard")?,
            len: 0,
        };
        let last = Instant::now();
        Ok(Self { pipe, kept, last })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.pipe.as_fd()
    }

    /// When this copy counts as stalled if nothing more arrives.
    pub fn deadline(&self) -> Instant {
        self.last + STALL
    }

    /// Move what is there. Ok(true) once the app closes its end.
    /// shortcut: an app killed mid-copy also looks like a clean end (the protocol
    /// has no length), so a truncated part is kept; no fix without a length.
    pub fn step(&mut self) -> io::Result<bool> {
        let k = &mut self.kept;
        loop {
            let room = (MAX_BYTES + 1 - k.len).min(CHUNK) as usize; // +1: notice overflow
            match splice::pipe_to_file(self.pipe.as_fd(), k.fd.as_fd(), &mut k.len, room) {
                Ok(0) => return Ok(true),
                Ok(_) if k.len > MAX_BYTES => return Err(io::Error::other("copy over 64 MiB")),
                Ok(_) => self.last = Instant::now(),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// The finished copy; None if it was empty.
    pub fn finish(self) -> Option<Kept> {
        (self.kept.len > 0).then_some(self.kept)
    }
}

/// Feeds the kept copy to one paste.
pub struct Writer {
    out: File,
    /// The paste fd is a pipe (every real app): splice into it without O_NONBLOCK.
    pipe: bool,
    data: Rc<Kept>,
    off: i64,
}

impl Writer {
    pub fn new(out: OwnedFd, data: Rc<Kept>) -> io::Result<Self> {
        let out = File::from(out);
        let pipe = out.metadata()?.file_type().is_fifo();
        if !pipe {
            // shortcut: O_NONBLOCK lives on the open file, which the app may share
            // (its own stdout); only odd apps pass non-pipes. Fix if one shows up.
            sys::set_nonblocking(out.as_fd())?;
        }
        Ok(Self {
            out,
            pipe,
            data,
            off: 0,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.out.as_fd()
    }

    /// Send what fits. Ok(true) once everything is out.
    pub fn step(&mut self) -> io::Result<bool> {
        let (file, out) = (self.data.fd.as_fd(), self.out.as_fd());
        let send = if self.pipe {
            splice::file_to_pipe
        } else {
            splice::file_to_fd
        };
        while self.off < self.data.len {
            let left = (self.data.len - self.off).min(CHUNK) as usize;
            match send(file, out, &mut self.off, left) {
                Ok(0) => break, // memfd shorter than recorded; can't happen, don't spin
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::FileExt;

    fn contents(k: &Kept) -> Vec<u8> {
        let mut buf = vec![0; k.len as usize];
        let file = File::from(k.fd.try_clone().unwrap());
        file.read_exact_at(&mut buf, 0).unwrap();
        buf
    }

    fn kept(data: &[u8]) -> Rc<Kept> {
        let fd = splice::memfd(c"test").unwrap();
        File::from(fd.try_clone().unwrap()).write_all(data).unwrap();
        let len = data.len() as i64;
        Rc::new(Kept { fd, len })
    }

    #[test]
    fn reader_waits_then_finishes() {
        let (r, w) = sys::pipe().unwrap();
        let mut reader = Reader::new(r).unwrap();
        assert!(!reader.step().unwrap());
        let mut w = File::from(w);
        w.write_all(b"hello ").unwrap();
        assert!(!reader.step().unwrap());
        w.write_all(b"world").unwrap();
        drop(w);
        assert!(reader.step().unwrap());
        assert_eq!(contents(&reader.finish().unwrap()), b"hello world");
    }

    #[test]
    fn empty_copy_keeps_nothing() {
        let (r, w) = sys::pipe().unwrap();
        drop(w);
        let mut reader = Reader::new(r).unwrap();
        assert!(reader.step().unwrap());
        assert!(reader.finish().is_none());
    }

    #[test]
    fn roundtrip_through_a_full_blocking_pipe() {
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        // The write end stays blocking: a hang here means splice ignored NONBLOCK.
        let (r, w) = sys::pipe().unwrap();
        let mut writer = Some(Writer::new(w, kept(&data)).unwrap());
        assert!(!writer.as_mut().unwrap().step().unwrap()); // 3 MB > any pipe buffer
        let mut reader = Reader::new(r).unwrap();
        while !reader.step().unwrap() {
            if writer.as_mut().is_some_and(|w| w.step().unwrap()) {
                writer = None; // closing the write end lets the reader see EOF
            }
        }
        assert_eq!(contents(&reader.finish().unwrap()), data);
    }

    #[test]
    fn writer_serves_a_non_pipe() {
        let out = splice::memfd(c"out").unwrap();
        let file = File::from(out.try_clone().unwrap());
        let mut writer = Writer::new(out, kept(b"to a file")).unwrap();
        assert!(writer.step().unwrap());
        let mut buf = [0; 9];
        file.read_exact_at(&mut buf, 0).unwrap();
        assert_eq!(&buf, b"to a file");
    }

    #[test]
    fn writer_reports_a_closed_reader() {
        let (r, w) = sys::pipe().unwrap();
        drop(r);
        let mut writer = Writer::new(w, kept(b"abc")).unwrap();
        let err = writer.step().unwrap_err(); // Rust ignores SIGPIPE
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
