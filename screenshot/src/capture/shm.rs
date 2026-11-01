//! Pixel memory the compositor copies into: an anonymous file, mapped into us.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use crate::sys;

/// `len` bytes of shared memory. Unmapped on drop.
pub struct Map {
    fd: OwnedFd,
    ptr: *mut u8,
    len: usize,
}

impl Map {
    pub fn new(len: usize) -> io::Result<Self> {
        let fd = sys::memfd()?;
        std::fs::File::from(fd.try_clone()?).set_len(len as u64)?;
        let ptr = sys::map_shared(fd.as_fd(), len)?;
        Ok(Self { fd, ptr, len })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Fault every page in (as huge pages) now, keeping what is already written.
    pub fn populate(&self) -> io::Result<()> {
        sys::populate(self.ptr, self.len)
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` is a live mapping of `len` bytes owned by this value. The
        // compositor only writes before `ready`, and we only read after it.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

// SAFETY: after `ready` the mapping is only read (`bytes` hands out `&[u8]`), so
// sharing it between encoder threads is like sharing a `&[u8]`.
unsafe impl Sync for Map {}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` are the mapping made in `new`, unused after this.
        unsafe { sys::unmap(self.ptr, self.len) };
    }
}
