//! Non-blocking transfers, so a slow or stuck app never stalls the daemon: a
//! `Reader` splices a new copy into a memfd, a `Writer` sendfiles it to a paste.
//! The bytes never pass through our heap.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::rc::Rc;

use crate::sys::{self, splice};

/// Bigger copies are dropped rather than kept in memory.
pub const MAX_BYTES: i64 = 64 << 20;
/// Bytes moved per syscall; the kernel moves less if less is there.
const CHUNK: usize = 1 << 20;

/// One kept copy: a memfd nobody writes to again, and its length.
pub struct Kept {
    fd: OwnedFd,
    len: i64,
}

/// Pulls one copy out of the app that made it.
pub struct Reader {
    pipe: OwnedFd,
    kept: Kept,
}

impl Reader {
    pub fn new(pipe: OwnedFd) -> io::Result<Self> {
        sys::set_nonblocking(pipe.as_fd())?;
        let _ = splice::grow_pipe(pipe.as_fd()); // only a speedup; 64 KiB works too
        let kept = Kept {
            fd: splice::memfd(c"clip")?,
            len: 0,
        };
        Ok(Self { pipe, kept })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.pipe.as_fd()
    }

    /// Move what is there. Ok(true) once the app closes its end.
    pub fn step(&mut self) -> io::Result<bool> {
        let k = &mut self.kept;
        loop {
            match splice::pipe_to_file(self.pipe.as_fd(), k.fd.as_fd(), &mut k.len, CHUNK) {
                Ok(0) => return Ok(true),
                Ok(_) if k.len > MAX_BYTES => return Err(io::Error::other("copy over 64 MiB")),
                Ok(_) => {}
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
    out: OwnedFd,
    data: Rc<Kept>,
    off: i64,
}

impl Writer {
    pub fn new(out: OwnedFd, data: Rc<Kept>) -> io::Result<Self> {
        sys::set_nonblocking(out.as_fd())?;
        Ok(Self { out, data, off: 0 })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.out.as_fd()
    }

    /// Send what fits. Ok(true) once everything is out.
    pub fn step(&mut self) -> io::Result<bool> {
        let (file, out) = (self.data.fd.as_fd(), self.out.as_fd());
        while self.off < self.data.len {
            let left = (self.data.len - self.off) as usize;
            match splice::file_to_fd(file, out, &mut self.off, left.min(CHUNK)) {
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
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::FileExt;

    fn contents(k: &Kept) -> Vec<u8> {
        let mut buf = vec![0; k.len as usize];
        File::from(k.fd.try_clone().unwrap())
            .read_exact_at(&mut buf, 0)
            .unwrap();
        buf
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
    fn roundtrip_through_a_full_pipe() {
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let fd = splice::memfd(c"test").unwrap();
        File::from(fd.try_clone().unwrap())
            .write_all(&data)
            .unwrap();
        let kept = Rc::new(Kept {
            fd,
            len: data.len() as i64,
        });
        let (r, w) = sys::pipe().unwrap();
        let mut writer = Some(Writer::new(w, kept).unwrap());
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
    fn writer_reports_a_closed_reader() {
        let fd = splice::memfd(c"test").unwrap();
        File::from(fd.try_clone().unwrap())
            .write_all(b"abc")
            .unwrap();
        let (r, w) = sys::pipe().unwrap();
        drop(r);
        let mut writer = Writer::new(w, Rc::new(Kept { fd, len: 3 })).unwrap();
        assert!(writer.step().is_err()); // EPIPE; Rust ignores SIGPIPE
    }
}
