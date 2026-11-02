//! Move a file to the home trash as the FreeDesktop trash spec lays it out, so the
//! `files` app, Nemo and GTK apps see it and can restore it:
//! `$XDG_DATA_HOME/Trash/files/<name>` holds the file, `Trash/info/<name>.trashinfo`
//! says where it came from and when.
//! Copied from ../files/src/ops/trash.rs (put only) with `local` from ../files/src/sys.rs
//! and `encode` from ../files/src/uri.rs, at files ebf1682.
//! shortcut: home trash only; files on another drive fail with a clear error
//! (the spec's `$topdir/.Trash-$uid` is the upgrade when that bites).

use std::ffi::{c_char, c_int, c_long};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    data.join("Trash")
}

/// Move `p` to the trash.
pub fn put(p: &Path) -> io::Result<()> {
    put_in(&root(), p).map(drop)
}

/// Returns the name the file got in the trash.
fn put_in(root: &Path, p: &Path) -> io::Result<String> {
    let (files, info) = (root.join("files"), root.join("info"));
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let abs = std::path::absolute(p)?;
    let base = abs
        .file_name()
        .ok_or(io::ErrorKind::InvalidInput)?
        .to_string_lossy()
        .into_owned();
    // Claim a name by creating its info file exclusively; a race just tries the next.
    for n in 1.. {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base}.{n}")
        };
        if files.join(&name).symlink_metadata().is_ok() {
            continue;
        }
        let info_path = info.join(format!("{name}.trashinfo"));
        let mut f = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        f.write_all(info_text(&abs, now()).as_bytes())?;
        if let Err(e) = fs::rename(&abs, files.join(&name)) {
            let _ = fs::remove_file(&info_path);
            return Err(if e.raw_os_error() == Some(EXDEV) {
                io::Error::other("can't trash files on another drive yet")
            } else {
                e
            });
        }
        return Ok(name);
    }
    unreachable!("1.. never ends")
}

/// linux errno: cross-device link.
const EXDEV: i32 = 18;

fn info_text(abs: &Path, t: Local) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
        encode(abs),
        t.year,
        t.month + 1,
        t.day,
        t.hour,
        t.min,
        t.sec
    )
}

/// Percent-encode a path's bytes (RFC 3986 unreserved and `/` stay).
fn encode(p: &Path) -> String {
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"-._~/".contains(&b);
    p.as_os_str()
        .as_bytes()
        .iter()
        .map(|&b| {
            if plain(b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Local time, month 0-based.
#[derive(Clone, Copy)]
struct Local {
    year: c_int,
    month: u8,
    day: u8,
    hour: u8,
    min: u8,
    sec: u8,
}

/// bits/types/struct_tm.h: struct tm (x86_64 glibc).
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
    // time.h: struct tm *localtime_r(const time_t *timer, struct tm *tp);
    fn localtime_r(timer: *const i64, tp: *mut Tm) -> *mut Tm;
}

/// Now, local time (honors TZ and /etc/localtime), as the spec wants.
fn now() -> Local {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
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
    fn puts_in_a_scratch_trash_without_clobbering() {
        let base = std::env::temp_dir().join(format!("image-trash-{}", std::process::id()));
        let (root, home) = (base.join("Trash"), base.join("home"));
        fs::create_dir_all(&home).unwrap();
        let item = home.join("a b.png");
        fs::write(&item, "x").unwrap();
        let n1 = put_in(&root, &item).unwrap();
        fs::write(&item, "y").unwrap();
        let n2 = put_in(&root, &item).unwrap();
        let gone = item.exists();
        let first = fs::read_to_string(root.join("files").join(&n1)).unwrap();
        let info = fs::read_to_string(root.join("info/a b.png.trashinfo")).unwrap();
        fs::remove_dir_all(&base).unwrap();
        assert_eq!((n1.as_str(), n2.as_str()), ("a b.png", "a b.png.2"));
        assert!(!gone);
        assert_eq!(first, "x");
        assert!(info.contains("/home/a%20b.png\nDeletionDate="), "{info}");
    }

    #[test]
    fn layout_matches_glibc() {
        assert_eq!(size_of::<Tm>(), 56);
    }
}
