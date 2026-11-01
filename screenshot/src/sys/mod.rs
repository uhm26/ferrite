//! The few libc calls std doesn't wrap, declared by hand (no `libc` crate).
//! Signatures from /usr/include (glibc, x86_64). poll/check from ../clip/src/sys/mod.rs,
//! memfd/mmap from ../wall/src/sys.rs (2026-11-01); madvise and the clock are new.

pub mod sock;

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

// poll.h
const POLLIN: i16 = 1;
// sys/mman.h, linux/memfd.h
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MFD_CLOEXEC: c_uint = 1;
const MADV_HUGEPAGE: c_int = 14;
const MADV_POPULATE_WRITE: c_int = 23;

/// poll.h: struct pollfd
#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

/// time.h: struct tm (56 bytes on x86_64 glibc). Only libc reads its fields.
#[repr(C, align(8))]
struct Tm([u8; 56]);

unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
    fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
    fn time(t: *mut i64) -> i64;
    fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    fn strftime(s: *mut c_char, max: usize, fmt: *const c_char, tm: *const Tm) -> usize;
}

/// For libc-style calls that return -1 and set errno (kit/ffi.rs).
fn check(ret: c_int) -> io::Result<c_int> {
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// Sleep until `fd` is readable. EINTR counts as "woke up".
pub fn wait_readable(fd: BorrowedFd) -> io::Result<()> {
    let mut p = PollFd {
        fd: fd.as_raw_fd(),
        events: POLLIN,
        revents: 0,
    };
    // SAFETY: `p` is one valid, exclusively borrowed pollfd.
    match check(unsafe { poll(&mut p, 1, -1) }) {
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(()),
        r => r.map(|_| ()),
    }
}

/// An anonymous in-memory file (close-on-exec).
pub fn memfd() -> io::Result<OwnedFd> {
    // SAFETY: the name is a valid NUL-terminated string; flags are valid.
    let fd = check(unsafe { memfd_create(c"snap".as_ptr(), MFD_CLOEXEC) })?;
    // SAFETY: memfd_create returned a fresh fd that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Map `len` bytes of `fd` shared, read-write, asking for huge pages (shmem THP is
/// "advise" on Arch: one fault per 2 MiB instead of per 4 KiB, for us and the compositor).
pub fn map_shared(fd: BorrowedFd, len: usize) -> io::Result<*mut u8> {
    // SAFETY: plain mmap of a file we own; the result is checked below.
    let p = unsafe {
        mmap(
            std::ptr::null_mut(),
            len,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if p as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `p` is the live mapping of `len` bytes just made. Failure is harmless.
    unsafe { madvise(p, len, MADV_HUGEPAGE) };
    Ok(p.cast())
}

/// Allocate the pages of a `map_shared` mapping now, keeping any data already there.
pub fn populate(ptr: *mut u8, len: usize) -> io::Result<()> {
    // SAFETY: MADV_POPULATE_WRITE only faults pages in; it never changes contents.
    check(unsafe { madvise(ptr.cast(), len, MADV_POPULATE_WRITE) }).map(|_| ())
}

/// # Safety
/// `ptr`/`len` must come from one `map_shared` call and not be used afterwards.
pub unsafe fn unmap(ptr: *mut u8, len: usize) {
    // SAFETY: caller guarantees this is a live mapping of exactly `len` bytes.
    unsafe { munmap(ptr.cast(), len) };
}

/// The local time formatted with strftime `fmt`.
pub fn local_time(fmt: &std::ffi::CStr) -> String {
    let mut tm = Tm([0; 56]);
    let mut buf = [0u8; 64];
    // SAFETY: time(NULL) only returns; localtime_r writes into our `tm`; strftime
    // writes at most `buf.len()` bytes and returns how many (0 on overflow).
    let n = unsafe {
        let now = time(std::ptr::null_mut());
        localtime_r(&now, &mut tm);
        strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &tm)
    };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_formats() {
        assert_eq!(local_time(c"%Y").len(), 4);
    }
}
