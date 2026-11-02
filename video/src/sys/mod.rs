//! Raw C interfaces, one module per library. Safe wrappers live in `gpu.rs` and
//! `player.rs`.

pub mod egl;
pub mod gbm;
pub mod mpv;

use std::ffi::{c_int, c_void};

unsafe extern "C" {
    // unistd.h: ssize_t write(int fd, const void *buf, size_t count)
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
}

/// Wake the main loop: one byte into the (non-blocking) pipe whose write end is
/// `d`. Called by mpv from its own threads.
///
/// # Safety
/// `d` must be a pipe fd cast to a pointer, open for as long as mpv may call this.
pub unsafe extern "C" fn wake(d: *mut c_void) {
    let byte = 1u8;
    // SAFETY: one byte from a live stack value; a full pipe (EAGAIN) is fine: the
    // loop is awake already.
    unsafe { write(d as usize as c_int, (&raw const byte).cast(), 1) };
}
