//! Watching folders for changed files (inotify): the event loop polls the fd and
//! reloads settings when it becomes readable. No timers, no work while idle.
//! Constants and signatures from /usr/include/sys/inotify.h (glibc 2.44).

use std::ffi::{CString, c_char, c_int};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::check;

const IN_NONBLOCK: c_int = 0o4000;
const IN_CLOEXEC: c_int = 0o2_000_000;
const IN_CLOSE_WRITE: u32 = 0x8;
const IN_MOVED_TO: u32 = 0x80;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;

unsafe extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
}

pub struct Watch(File);

impl Watch {
    /// Watch `dirs` (created if missing) for files written, moved in or removed.
    /// Folders, not files: editors often save by renaming a new file over the old.
    pub fn new(dirs: &[&Path]) -> io::Result<Self> {
        // SAFETY: flags are valid; the fd is checked and then owned by us.
        let fd = check(unsafe { inotify_init1(IN_NONBLOCK | IN_CLOEXEC) })?;
        // SAFETY: inotify_init1 returned a fresh fd nothing else owns.
        let file = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        let mask = IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE | IN_DELETE;
        for dir in dirs {
            std::fs::create_dir_all(dir)?;
            let path = CString::new(dir.as_os_str().as_bytes())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            // SAFETY: valid fd and NUL-terminated path.
            check(unsafe { inotify_add_watch(file.as_raw_fd(), path.as_ptr(), mask) })?;
        }
        Ok(Watch(file))
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }

    /// Read all pending events. True if anything changed.
    pub fn drain(&mut self) -> bool {
        let mut buf = [0u8; 4096];
        let mut any = false;
        while let Ok(n) = self.0.read(&mut buf) {
            if n == 0 {
                break;
            }
            any = true;
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_a_written_file() {
        let dir = std::env::temp_dir().join(format!("term-watch-{}", std::process::id()));
        let mut w = Watch::new(&[&dir]).unwrap();
        assert!(!w.drain());
        std::fs::write(dir.join("x"), "1").unwrap();
        assert!(w.drain());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
