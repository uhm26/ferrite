//! Pixel memory the compositor can read: an anonymous file, mapped into us.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};

use crate::sys;

/// `len` bytes of shared memory, mapped read-write. Unmapped on drop; the compositor
/// keeps its own mapping, so dropping this never changes what is on screen.
pub struct Pixels {
    file: File,
    ptr: *mut u8,
    len: usize,
}

impl Pixels {
    pub fn new(len: usize) -> io::Result<Self> {
        let file = File::from(sys::memfd()?);
        file.set_len(len as u64)?;
        let ptr = sys::map_shared(file.as_fd(), len)?;
        Ok(Self { file, ptr, len })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }

    pub fn as_u32_mut(&mut self) -> &mut [u32] {
        // SAFETY: `ptr` is a live, page-aligned mapping of `len` bytes that only this
        // value owns; `len` is a multiple of 4 (whole XRGB pixels).
        unsafe { std::slice::from_raw_parts_mut(self.ptr.cast(), self.len / 4) }
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` are the mapping made in `new`, unused after this.
        unsafe { sys::unmap(self.ptr, self.len) };
    }
}
