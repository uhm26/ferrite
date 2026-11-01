//! The few libc calls std doesn't wrap, declared by hand (no `libc` crate).
//! Signatures from /usr/include (glibc 2.44, x86_64). Started as a copy of
//! ../wall/src/sys.rs (2026-11-01); `check` is from kit/ffi.rs (2026-10-31).

pub mod mem;
pub mod pty;
pub mod sock;
pub mod watch;

use std::ffi::{c_char, c_int};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

// poll.h
pub const POLLIN: i16 = 1;
pub const POLLOUT: i16 = 4;
pub const POLLERR: i16 = 8;
pub const POLLHUP: i16 = 16;
// fcntl.h
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const O_NONBLOCK: c_int = 0o4000;
const O_CLOEXEC: c_int = 0o2_000_000;
const F_GETFD: c_int = 1;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: c_int = 1;
// signal.h
const SIGKILL: c_int = 9;
// locale.h
const LC_CTYPE: c_int = 0;

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
    // malloc.h (glibc)
    fn malloc_trim(pad: usize) -> c_int;
    // locale.h, wchar.h
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
    fn wcwidth(c: u32) -> c_int;
    fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
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

    pub fn ready(&self, mask: i16) -> bool {
        self.revents & mask != 0
    }
}

/// Wait until one of `fds` is ready or `timeout_ms` passes (-1 = forever).
/// EINTR counts as "nothing ready".
pub fn wait(fds: &mut [PollFd], timeout_ms: i32) -> io::Result<()> {
    // SAFETY: `fds` is a valid, exclusively borrowed array of pollfd.
    match check(unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, timeout_ms) }) {
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

/// Whether `fd` is closed on exec. Off = it survives into an exec'd program
/// (how a restart in place keeps the shells' ptys).
pub fn set_cloexec(fd: BorrowedFd, on: bool) -> io::Result<()> {
    let fd = fd.as_raw_fd();
    // SAFETY: F_GETFD/F_SETFD on an fd we borrow; no pointers involved.
    let flags = check(unsafe { fcntl(fd, F_GETFD) })?;
    let flags = if on { flags | FD_CLOEXEC } else { flags & !FD_CLOEXEC };
    // SAFETY: as above.
    check(unsafe { fcntl(fd, F_SETFD, flags) }).map(|_| ())
}

/// Own an fd number we inherited through exec, if it is really open.
pub fn adopt_fd(raw: c_int) -> Option<OwnedFd> {
    // SAFETY: F_GETFD only checks that the descriptor exists.
    let open = unsafe { fcntl(raw, F_GETFD) } != -1;
    // SAFETY: the exec'ing process handed this fd to us and nothing else owns it.
    open.then(|| unsafe { OwnedFd::from_raw_fd(raw) })
}

/// SIGKILL our child `pid` and wait for it, so no zombie stays.
pub fn kill_and_reap(pid: u32) {
    let pid = pid as c_int;
    let mut status = 0;
    // SAFETY: plain syscalls on a pid we spawned; `status` is a valid out-pointer.
    unsafe {
        kill(pid, SIGKILL);
        waitpid(pid, &mut status, 0);
    }
}

/// A close-on-exec pipe: (read end, write end).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as c_int; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    check(unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC) })?;
    // SAFETY: pipe2 just created both fds; we are their only owner.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Give freed heap memory back to the OS (glibc keeps it otherwise).
pub fn trim_heap() {
    // SAFETY: malloc_trim only walks glibc's own heap; always safe to call.
    unsafe { malloc_trim(0) };
}

/// Make `wcwidth` know Unicode. C.UTF-8 is built into glibc ≥ 2.35, so this
/// works whatever `LANG` says. Call once at startup, before any threads.
pub fn init_locale() {
    // SAFETY: valid NUL-terminated string; called before other threads exist.
    unsafe { setlocale(LC_CTYPE, c"C.UTF-8".as_ptr()) };
}

/// Terminal columns a character takes: 0 (combining), 1 or 2. Unknown → 1.
pub fn char_width(c: char) -> usize {
    if (c as u32) < 0x300 {
        return 1;
    }
    // SAFETY: wcwidth reads only its argument and the locale tables.
    match unsafe { wcwidth(c as u32) } {
        0 => 0,
        2 => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_glibc() {
        assert_eq!(size_of::<PollFd>(), 8);
    }

    #[test]
    fn widths() {
        init_locale();
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('漢'), 2);
        assert_eq!(char_width('\u{301}'), 0);
        assert_eq!(char_width('─'), 1);
    }
}
