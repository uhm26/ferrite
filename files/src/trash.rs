//! The home trash, as the FreeDesktop trash spec lays it out, so Nemo, GTK apps and
//! we share one: `$XDG_DATA_HOME/Trash/files/<name>` holds the item and
//! `Trash/info/<name>.trashinfo` says where it came from and when.
//! shortcut: home trash only; items on another filesystem fail with a clear error
//! (the spec's `$topdir/.Trash-$uid` is the upgrade when that bites).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::{sys, uri};

fn root() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::view::home().join(".local/share"));
    data.join("Trash")
}

/// Where trashed items live (what the Trash place shows).
pub fn files_dir() -> PathBuf {
    root().join("files")
}

/// Move `p` to the trash. Returns the name it got there.
pub fn put(p: &Path) -> io::Result<String> {
    put_in(&root(), p)
}

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
                io::Error::other("can't trash items on another drive yet")
            } else {
                e
            });
        }
        return Ok(name);
    }
    unreachable!("1.. never ends")
}

/// Put the trashed item `name` back where it came from (its folder is made again if
/// gone). Fails if something now sits at that path.
pub fn restore(name: &str) -> io::Result<PathBuf> {
    restore_in(&root(), name)
}

fn restore_in(root: &Path, name: &str) -> io::Result<PathBuf> {
    let info_path = root.join("info").join(format!("{name}.trashinfo"));
    let text = fs::read_to_string(&info_path)?;
    let to = original(&text).ok_or_else(|| io::Error::other("trash info has no Path"))?;
    if to.symlink_metadata().is_ok() {
        return Err(io::Error::other(format!("{} exists already", to.display())));
    }
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::rename(root.join("files").join(name), &to)?;
    fs::remove_file(info_path)?;
    Ok(to)
}

/// Delete the trashed item `name` for good.
pub fn erase(name: &str) -> io::Result<()> {
    let p = files_dir().join(name);
    if p.symlink_metadata()?.is_dir() {
        fs::remove_dir_all(&p)?;
    } else {
        fs::remove_file(&p)?;
    }
    let _ = fs::remove_file(root().join("info").join(format!("{name}.trashinfo")));
    Ok(())
}

/// linux errno: cross-device link.
const EXDEV: i32 = 18;

fn now() -> sys::Local {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    sys::local(secs)
}

fn info_text(abs: &Path, t: sys::Local) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
        uri::encode(abs),
        t.year,
        t.month + 1,
        t.day,
        t.hour,
        t.min,
        t.sec
    )
}

fn original(info: &str) -> Option<PathBuf> {
    info.lines()
        .find_map(|l| l.strip_prefix("Path="))
        .map(uri::decode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_restore_in_a_scratch_trash() {
        let base = std::env::temp_dir().join(format!("files-trash-{}", std::process::id()));
        let (root, home) = (base.join("Trash"), base.join("home"));
        fs::create_dir_all(&home).unwrap();
        let item = home.join("a.txt");
        fs::write(&item, "x").unwrap();
        let n1 = put_in(&root, &item).unwrap();
        fs::write(&item, "y").unwrap();
        let n2 = put_in(&root, &item).unwrap();
        let gone = item.exists();
        let back = restore_in(&root, &n1).unwrap();
        let clash = restore_in(&root, &n2).is_err();
        let content = fs::read_to_string(&item).unwrap();
        fs::remove_dir_all(&base).unwrap();
        assert_eq!((n1.as_str(), n2.as_str()), ("a.txt", "a.txt.2"));
        assert!(!gone);
        assert_eq!((back, content.as_str()), (item, "x"));
        assert!(clash, "restoring over an existing file fails");
    }

    #[test]
    fn info_round_trips_the_path() {
        let t = sys::Local {
            year: 2026,
            month: 10,
            day: 1,
            hour: 9,
            min: 5,
            sec: 7,
        };
        let text = info_text(Path::new("/home/g/a b.txt"), t);
        assert_eq!(
            text,
            "[Trash Info]\nPath=/home/g/a%20b.txt\nDeletionDate=2026-11-01T09:05:07\n"
        );
        assert_eq!(original(&text), Some(PathBuf::from("/home/g/a b.txt")));
    }
}
