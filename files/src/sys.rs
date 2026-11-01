//! The two libc calls std doesn't offer: free disk space (statvfs) and local time
//! (localtime_r). Own `extern "C"`, no libc crate (rust-deps skill). x86_64 glibc layouts.

use std::ffi::{CString, c_char, c_int, c_long, c_ulong};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// bits/statvfs.h: struct statvfs (64-bit: no __f_unused).
#[repr(C)]
#[derive(Default)]
struct StatVfs {
    f_bsize: c_ulong,
    f_frsize: c_ulong,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: c_ulong,
    f_flag: c_ulong,
    f_namemax: c_ulong,
    f_type: u32,
    f_spare: [c_int; 5],
}

/// bits/types/struct_tm.h: struct tm.
#[repr(C)]
struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

unsafe extern "C" {
    // sys/statvfs.h: int statvfs(const char *file, struct statvfs *buf);
    fn statvfs(file: *const c_char, buf: *mut StatVfs) -> c_int;
    // time.h: struct tm *localtime_r(const time_t *timer, struct tm *tp);
    fn localtime_r(timer: *const i64, tp: *mut Tm) -> *mut Tm;
}

/// Bytes free for us (not root) and total on the filesystem holding `path`.
pub fn disk(path: &Path) -> io::Result<(u64, u64)> {
    let c = CString::new(path.as_os_str().as_bytes())?;
    let mut buf = StatVfs::default();
    // SAFETY: `c` is a valid C string, `buf` a writable struct of the header's layout.
    if unsafe { statvfs(c.as_ptr(), &raw mut buf) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((buf.f_bavail * buf.f_frsize, buf.f_blocks * buf.f_frsize))
}

/// A moment in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 0 = January.
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub min: u8,
    pub sec: u8,
}

/// Unix seconds → local time (honors TZ and /etc/localtime).
pub fn local(secs: i64) -> Local {
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 1,
        tm_mon: 0,
        tm_year: 70,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: both pointers are valid for the call; on failure tm stays the epoch.
    unsafe { localtime_r(&raw const secs, &raw mut tm) };
    Local {
        year: tm.tm_year + 1900,
        month: tm.tm_mon as u8,
        day: tm.tm_mday as u8,
        hour: tm.tm_hour as u8,
        min: tm.tm_min as u8,
        sec: tm.tm_sec as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_glibc() {
        assert_eq!(size_of::<StatVfs>(), 112);
        assert_eq!(size_of::<Tm>(), 56);
    }

    #[test]
    fn calls_work() {
        let (free, total) = disk(Path::new("/")).unwrap();
        assert!(free > 0 && total >= free);
        assert!(local(1_800_000_000).year >= 2027); // Jan 2027 in any zone
    }
}
