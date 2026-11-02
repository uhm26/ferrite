//! The few libc calls std doesn't wrap: memfd + mmap for shm buffers, sendmsg for
//! passing a file descriptor over the Wayland socket, poll for the event loop.
//! Signatures from /usr/include (glibc, x86_64). `check` is from kit/ffi.rs (2026-10-31).

use std::ffi::{c_int, c_uint, c_void};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

// sys/mman.h, linux/memfd.h
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MFD_CLOEXEC: c_uint = 1;
// sys/socket.h, bits/socket.h
const MSG_NOSIGNAL: c_int = 0x4000;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
// poll.h
pub const POLLIN: i16 = 1;

#[repr(C)]
struct IoVec {
    base: *const c_void,
    len: usize,
}

// bits/socket.h: struct msghdr (size_t iovlen/controllen on x86_64 glibc)
#[repr(C)]
struct MsgHdr {
    name: *mut c_void,
    namelen: u32,
    iov: *const IoVec,
    iovlen: usize,
    control: *mut c_void,
    controllen: usize,
    flags: c_int,
}

// bits/socket.h: struct cmsghdr, followed by one fd
#[repr(C)]
struct CmsgFd {
    len: usize,
    level: c_int,
    kind: c_int,
    fd: c_int,
    _pad: c_int, // CMSG_SPACE(4) = 24 on x86_64
}

// poll.h: struct pollfd
#[repr(C)]
pub struct PollFd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

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
    fn sendmsg(fd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
    // malloc.h (glibc)
    #[cfg(target_env = "gnu")]
    fn malloc_trim(pad: usize) -> c_int;
    #[cfg(target_env = "gnu")]
    fn mallopt(param: c_int, value: c_int) -> c_int;
}

/// For libc-style calls that return -1 and set errno (kit/ffi.rs).
fn check(ret: c_int) -> io::Result<c_int> {
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// An anonymous in-memory file (close-on-exec).
pub fn memfd() -> io::Result<OwnedFd> {
    // SAFETY: the name is a valid NUL-terminated string; flags are valid.
    let fd = check(unsafe { memfd_create(c"wallpaper".as_ptr(), MFD_CLOEXEC) })?;
    // SAFETY: memfd_create returned a fresh fd that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Map `len` bytes of `fd` shared, read-write.
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
    Ok(p.cast())
}

/// # Safety
/// `ptr`/`len` must come from one `map_shared` call and not be used afterwards.
pub unsafe fn unmap(ptr: *mut u8, len: usize) {
    // SAFETY: caller guarantees this is a live mapping of exactly `len` bytes.
    unsafe { munmap(ptr.cast(), len) };
}

/// Send `bytes` with one file descriptor attached. Returns bytes written.
pub fn send_with_fd(sock: RawFd, bytes: &[u8], fd: RawFd) -> io::Result<usize> {
    let iov = IoVec {
        base: bytes.as_ptr().cast(),
        len: bytes.len(),
    };
    let mut cmsg = CmsgFd {
        len: 20,
        level: SOL_SOCKET,
        kind: SCM_RIGHTS,
        fd,
        _pad: 0,
    };
    let msg = MsgHdr {
        name: std::ptr::null_mut(),
        namelen: 0,
        iov: &iov,
        iovlen: 1,
        control: (&raw mut cmsg).cast(),
        controllen: size_of::<CmsgFd>(),
        flags: 0,
    };
    // SAFETY: every pointer in `msg` points at locals that outlive the call.
    let n = unsafe { sendmsg(sock, &msg, MSG_NOSIGNAL) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// Give freed heap memory back to the OS (glibc keeps it otherwise).
/// glibc gives every thread that allocates its own heap (arena), kept for life. pix
/// scales on all cores, so that is ~2 MB of idle RSS (4.7 vs 3.1 MB measured).
/// One heap is enough: the workers allocate 2 small Vecs each per draw.
/// musl (static builds) has neither knob and needs neither: its allocator has no per-thread
/// heaps and hands freed memory back on its own. Both are no-ops there.
pub fn one_heap() {
    #[cfg(target_env = "gnu")]
    {
        const M_ARENA_MAX: c_int = -8; // malloc.h
        // SAFETY: mallopt only sets a glibc tuning knob; call before threads start.
        unsafe { mallopt(M_ARENA_MAX, 1) };
    }
}

pub fn trim_heap() {
    // SAFETY: malloc_trim only walks glibc's own heap; always safe to call.
    #[cfg(target_env = "gnu")]
    unsafe {
        malloc_trim(0)
    };
}

/// Wait until one of `fds` is readable. Retries on EINTR.
pub fn wait(fds: &mut [PollFd]) -> io::Result<()> {
    loop {
        // SAFETY: `fds` is a valid, exclusively borrowed array of pollfd.
        match check(unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, -1) }) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            r => return r.map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_glibc() {
        assert_eq!(size_of::<MsgHdr>(), 56);
        assert_eq!(size_of::<CmsgFd>(), 24); // CMSG_SPACE(sizeof(int))
        assert_eq!(size_of::<PollFd>(), 8);
    }

    #[test]
    fn memfd_maps() {
        let fd = memfd().unwrap();
        std::fs::File::from(fd.try_clone().unwrap())
            .set_len(4096)
            .unwrap();
        let p = map_shared(fd.as_fd(), 4096).unwrap();
        // SAFETY: fresh 4096-byte mapping.
        unsafe {
            *p = 7;
            assert_eq!(*p, 7);
            unmap(p, 4096);
        }
    }

    use std::os::fd::AsFd;
}
