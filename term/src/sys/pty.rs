//! Pseudo-terminal calls: openpty (in libc.so since glibc 2.34), window size,
//! and making the pty the child's controlling terminal.

use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use super::check;

// asm-generic/ioctls.h
const TIOCSCTTY: c_ulong = 0x540E;
const TIOCSWINSZ: c_ulong = 0x5414;

/// asm-generic/termios.h: struct winsize
#[repr(C)]
struct WinSize {
    row: u16,
    col: u16,
    xpixel: u16,
    ypixel: u16,
}

unsafe extern "C" {
    fn openpty(
        master: *mut c_int,
        slave: *mut c_int,
        name: *mut c_char,
        termp: *const c_void,
        winp: *const WinSize,
    ) -> c_int;
    fn ioctl(fd: c_int, req: c_ulong, ...) -> c_int;
    fn setsid() -> c_int;
}

/// A new pty pair (master, slave), both close-on-exec on our side.
pub fn open() -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut m, mut s) = (-1, -1);
    // SAFETY: out-pointers are valid locals; name/termp/winp may be null.
    check(unsafe {
        openpty(
            &mut m,
            &mut s,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    })?;
    // SAFETY: openpty returned two fresh fds that only we own.
    let (m, s) = unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };
    // openpty doesn't set close-on-exec: without it every shell inherits every
    // tab's master, and closing a tab never hangs up the programs in it.
    super::set_cloexec(m.as_fd(), true)?;
    super::set_cloexec(s.as_fd(), true)?;
    Ok((m, s))
}

/// Tell the pty (and so the shell, via SIGWINCH) its size.
pub fn set_size(fd: BorrowedFd, cols: u16, rows: u16, px: (u16, u16)) -> io::Result<()> {
    let ws = WinSize {
        row: rows,
        col: cols,
        xpixel: px.0,
        ypixel: px.1,
    };
    // SAFETY: TIOCSWINSZ reads one struct winsize from a valid pointer.
    check(unsafe { ioctl(fd.as_raw_fd(), TIOCSWINSZ, &ws) }).map(|_| ())
}

/// In the forked child, before exec: own session, stdin (the pty slave) as the
/// controlling terminal. Only async-signal-safe calls.
pub fn become_session_leader() -> io::Result<()> {
    // SAFETY: setsid takes no arguments; async-signal-safe.
    check(unsafe { setsid() })?;
    // SAFETY: TIOCSCTTY on fd 0 with arg 0; async-signal-safe.
    check(unsafe { ioctl(0, TIOCSCTTY, 0) }).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn open_and_size() {
        let (m, _s) = open().unwrap();
        set_size(m.as_fd(), 80, 24, (800, 480)).unwrap();
        assert_eq!(size_of::<WinSize>(), 8);
    }
}
