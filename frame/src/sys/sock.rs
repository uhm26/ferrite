//! Passing file descriptors over the Wayland socket (SCM_RIGHTS). `send_with_fd` is
//! copied from ../wall/src/sys.rs (2026-11-01); `recv_with_fds` is new.
//! Copied into frame from ../term/src/sys/sock.rs at term 7be66f5 (2026-11-01).

use std::ffi::{c_int, c_void};
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

// sys/socket.h, bits/socket.h
const MSG_DONTWAIT: c_int = 0x40;
const MSG_NOSIGNAL: c_int = 0x4000;
const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
/// sizeof(struct cmsghdr) on x86_64: len (size_t) + level + type.
const CMSG_HDR: usize = 16;

#[repr(C)]
struct IoVec {
    base: *mut c_void,
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

unsafe extern "C" {
    fn sendmsg(fd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
}

/// Send `bytes` with one file descriptor attached. Returns bytes written.
pub fn send_with_fd(sock: RawFd, bytes: &[u8], fd: RawFd) -> io::Result<usize> {
    let iov = IoVec {
        base: bytes.as_ptr().cast_mut().cast(),
        len: bytes.len(),
    };
    let mut cmsg = CmsgFd {
        len: CMSG_HDR + 4,
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
    // SAFETY: every pointer in `msg` points at locals that outlive the call;
    // sendmsg only reads the iov buffer.
    let n = unsafe { sendmsg(sock, &msg, MSG_NOSIGNAL) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// Non-blocking read into `buf`; received fds are appended to `fds`.
/// Returns bytes read (0 = peer closed).
pub fn recv_with_fds(sock: RawFd, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> io::Result<usize> {
    let iov = IoVec {
        base: buf.as_mut_ptr().cast(),
        len: buf.len(),
    };
    let mut control = [0u64; 16]; // 128 bytes, 8-aligned like cmsghdr
    let mut msg = MsgHdr {
        name: std::ptr::null_mut(),
        namelen: 0,
        iov: &iov,
        iovlen: 1,
        control: control.as_mut_ptr().cast(),
        controllen: size_of_val(&control),
        flags: 0,
    };
    // SAFETY: `msg` points at `iov`, `buf` and `control`, all live and writable.
    let n = unsafe { recvmsg(sock, &mut msg, MSG_DONTWAIT | MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if msg.controllen > 0 {
        // Rare (only events carrying an fd), so the copy to bytes is fine here.
        let bytes: Vec<u8> = control.iter().flat_map(|w| w.to_ne_bytes()).collect();
        take_fds(&bytes[..msg.controllen.min(bytes.len())], fds);
    }
    Ok(n as usize)
}

/// Walk the control messages; keep every SCM_RIGHTS fd.
fn take_fds(mut c: &[u8], fds: &mut Vec<OwnedFd>) {
    while c.len() >= CMSG_HDR {
        let len = usize::from_ne_bytes(c[..8].try_into().unwrap_or_default());
        let level = i32::from_ne_bytes(c[8..12].try_into().unwrap_or_default());
        let kind = i32::from_ne_bytes(c[12..16].try_into().unwrap_or_default());
        if len < CMSG_HDR || len > c.len() {
            return;
        }
        if level == SOL_SOCKET && kind == SCM_RIGHTS {
            for raw in c[CMSG_HDR..len].as_chunks::<4>().0 {
                let fd = i32::from_ne_bytes(*raw);
                // SAFETY: the kernel just installed this fd for us; we are its only owner.
                fds.push(unsafe { OwnedFd::from_raw_fd(fd) });
            }
        }
        c = &c[len.next_multiple_of(8).min(c.len())..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn struct_sizes_match_glibc() {
        assert_eq!(size_of::<MsgHdr>(), 56);
        assert_eq!(size_of::<CmsgFd>(), 24); // CMSG_SPACE(sizeof(int))
    }

    #[test]
    fn fd_roundtrip() {
        let (a, b) = UnixStream::pair().unwrap();
        let file = std::fs::File::open("/dev/null").unwrap();
        send_with_fd(a.as_raw_fd(), b"hi", file.as_raw_fd()).unwrap();
        let mut buf = [0u8; 8];
        let mut fds = Vec::new();
        let n = recv_with_fds(b.as_raw_fd(), &mut buf, &mut fds).unwrap();
        assert_eq!(&buf[..n], b"hi");
        assert_eq!(fds.len(), 1);
    }
}
