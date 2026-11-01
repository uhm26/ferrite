//! Zero-copy moves between fds: copies land in a memfd (a RAM-only file) via
//! splice and go out to pastes via sendfile, never passing through our heap.

use std::ffi::{CStr, c_char, c_int, c_uint};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use super::{check, fcntl};

// sys/mman.h, fcntl.h (linux/fcntl.h)
const MFD_CLOEXEC: c_uint = 1;
const SPLICE_F_NONBLOCK: c_uint = 2;
const F_SETPIPE_SZ: c_int = 1031;

unsafe extern "C" {
    fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    fn splice(
        fd_in: c_int,
        off_in: *mut i64,
        fd_out: c_int,
        off_out: *mut i64,
        len: usize,
        flags: c_uint,
    ) -> isize;
    fn sendfile(out_fd: c_int, in_fd: c_int, offset: *mut i64, count: usize) -> isize;
}

fn checked(n: isize) -> io::Result<usize> {
    usize::try_from(n).map_err(|_| io::Error::last_os_error())
}

/// A new, empty RAM-only file.
pub fn memfd(name: &CStr) -> io::Result<OwnedFd> {
    // SAFETY: `name` is NUL-terminated and outlives the call.
    let fd = check(unsafe { memfd_create(name.as_ptr(), MFD_CLOEXEC) })?;
    // SAFETY: memfd_create just made this fd; we are its only owner.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Move up to `len` bytes from `pipe` into `file` at `*off` (advanced). Never blocks.
/// Ok(0): the pipe's writer closed and nothing is left.
pub fn pipe_to_file(
    pipe: BorrowedFd,
    file: BorrowedFd,
    off: &mut i64,
    len: usize,
) -> io::Result<usize> {
    let (i, o) = (pipe.as_raw_fd(), file.as_raw_fd());
    let null = std::ptr::null_mut();
    // SAFETY: `off` is a valid, exclusive i64 for the call; fds are borrowed and open.
    checked(unsafe { splice(i, null, o, off, len, SPLICE_F_NONBLOCK) })
}

/// Move up to `len` bytes of `file` from `*off` (advanced) into `pipe`. Never blocks,
/// without setting O_NONBLOCK on the pipe (that flag is shared with the app).
pub fn file_to_pipe(
    file: BorrowedFd,
    pipe: BorrowedFd,
    off: &mut i64,
    len: usize,
) -> io::Result<usize> {
    let (i, o) = (file.as_raw_fd(), pipe.as_raw_fd());
    let null = std::ptr::null_mut();
    // SAFETY: `off` is a valid, exclusive i64 for the call; fds are borrowed and open.
    checked(unsafe { splice(i, off, o, null, len, SPLICE_F_NONBLOCK) })
}

/// Send up to `len` bytes of `file` from `*off` (advanced) to a non-pipe `out`
/// (socket, file). Fails with EINVAL on O_APPEND files.
pub fn file_to_fd(
    file: BorrowedFd,
    out: BorrowedFd,
    off: &mut i64,
    len: usize,
) -> io::Result<usize> {
    // SAFETY: `off` is a valid, exclusive i64 for the call; fds are borrowed and open.
    checked(unsafe { sendfile(out.as_raw_fd(), file.as_raw_fd(), off, len) })
}

/// Ask for a 1 MiB pipe buffer (the unprivileged max): 16× fewer wakeups on big copies.
pub fn grow_pipe(pipe: BorrowedFd) -> io::Result<()> {
    // SAFETY: F_SETPIPE_SZ on a borrowed fd; no pointers involved.
    check(unsafe { fcntl(pipe.as_raw_fd(), F_SETPIPE_SZ, 1 << 20) }).map(|_| ())
}
