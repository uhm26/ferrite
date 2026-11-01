//! Shared memory: memfd + mmap. For shm pixel buffers and the keymap the compositor
//! sends us. Copied from ../wall/src/sys.rs and shm.rs (2026-11-01).
//! Copied into frame from ../term/src/sys/mem.rs at term 7be66f5 (2026-11-01).

use std::ffi::{c_int, c_uint, c_void};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use super::check;

// sys/mman.h, linux/memfd.h
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MAP_PRIVATE: c_int = 2;
const MFD_CLOEXEC: c_uint = 1;

unsafe extern "C" {
    fn memfd_create(name: *const std::ffi::c_char, flags: c_uint) -> c_int;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

/// A live mapping, unmapped on drop.
pub struct Map {
    ptr: *mut u8,
    len: usize,
}

impl Map {
    fn new(fd: BorrowedFd, len: usize, prot: c_int, flags: c_int) -> io::Result<Self> {
        // SAFETY: plain mmap of an fd we borrow; the result is checked below.
        let p = unsafe { mmap(std::ptr::null_mut(), len, prot, flags, fd.as_raw_fd(), 0) };
        if p as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { ptr: p.cast(), len })
    }

    /// Read-only private view of `len` bytes of `fd` (a keymap).
    pub fn read_only(fd: BorrowedFd, len: usize) -> io::Result<Self> {
        Self::new(fd, len, PROT_READ, MAP_PRIVATE)
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` is a live mapping of `len` bytes owned by `self`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` are exactly the mapping made in `new`, unused after this.
        unsafe { munmap(self.ptr.cast(), self.len) };
    }
}

/// `len` bytes of shared memory the compositor can map too. Our mapping is dropped
/// with this value; the compositor keeps its own.
pub struct Pixels {
    fd: OwnedFd,
    map: Map,
}

impl Pixels {
    pub fn new(len: usize) -> io::Result<Self> {
        // SAFETY: the name is a valid NUL-terminated string; flags are valid.
        let fd = check(unsafe { memfd_create(c"frame".as_ptr(), MFD_CLOEXEC) })?;
        // SAFETY: memfd_create returned a fresh fd that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        std::fs::File::from(fd.try_clone()?).set_len(len as u64)?;
        let map = Map::new(fd.as_fd(), len, PROT_READ | PROT_WRITE, MAP_SHARED)?;
        Ok(Self { fd, map })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn as_u32_mut(&mut self) -> &mut [u32] {
        // SAFETY: the mapping is live, page-aligned, writable, owned only by `self`;
        // the length is whole XRGB pixels (callers allocate w * h * 4).
        unsafe { std::slice::from_raw_parts_mut(self.map.ptr.cast(), self.map.len / 4) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_map() {
        let mut p = Pixels::new(4096).unwrap();
        p.as_u32_mut()[10] = 7;
        assert_eq!(p.as_u32_mut()[10], 7);
        let ro = Map::read_only(p.fd(), 4096).unwrap();
        assert_eq!(ro.bytes()[40], 7);
    }
}
