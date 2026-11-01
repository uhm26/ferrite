//! The few libc calls std doesn't wrap, declared by hand (no `libc` crate).
//! Signatures from /usr/include (glibc 2.44, x86_64). Trimmed copy of
//! ../term/src/sys/mod.rs (2026-11-01); `check` is from kit/ffi.rs.

pub mod sock;
pub mod splice;

use std::ffi::c_int;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

// poll.h
pub const POLLIN: i16 = 1;
pub const POLLOUT: i16 = 4;
// fcntl.h
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const O_NONBLOCK: c_int = 0o4000;
const O_CLOEXEC: c_int = 0o2_000_000;

/// poll.h: struct pollfd
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PollFd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
}

/// For libc-style calls that return -1 and set errno (kit/ffi.rs).
pub fn check(ret: c_int) -> io::Result<c_int> {
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

impl PollFd {
    pub fn new(fd: BorrowedFd, events: i16) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            events,
            revents: 0,
        }
    }
}

/// Sleep until one of `fds` is ready (no timeout). EINTR counts as "woke up".
pub fn wait(fds: &mut [PollFd]) -> io::Result<()> {
    // SAFETY: `fds` is a valid, exclusively borrowed array of pollfd.
    match check(unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, -1) }) {
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(()),
        r => r.map(|_| ()),
    }
}

pub fn set_nonblocking(fd: BorrowedFd) -> io::Result<()> {
    let fd = fd.as_raw_fd();
    // SAFETY: F_GETFL/F_SETFL on an fd we borrow; no pointers involved.
    let flags = check(unsafe { fcntl(fd, F_GETFL) })?;
    // SAFETY: as above.
    check(unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) }).map(|_| ())
}

/// A close-on-exec pipe: (read end, write end).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as c_int; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    check(unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC) })?;
    // SAFETY: pipe2 just created both fds; we are their only owner.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_glibc() {
        assert_eq!(size_of::<PollFd>(), 8);
    }
}
