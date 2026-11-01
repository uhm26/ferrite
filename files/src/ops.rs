//! File operations. Long ones (copy, move, trash, restore, erase) run on a worker
//! thread so the window stays responsive; each reports one line through a pipe the
//! event loop polls. Quick ones (rename, new folder) run inline. Names never
//! collide: a copy next to its original becomes `a (copy).txt`, then `a (copy 2).txt`.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

use crate::trash;

/// What a worker does.
pub enum Op {
    /// Copy these into the folder.
    Copy(Vec<PathBuf>, PathBuf),
    /// Move these into the folder.
    Move(Vec<PathBuf>, PathBuf),
    Trash(Vec<PathBuf>),
    /// Trashed item names back to where they came from.
    Restore(Vec<String>),
    /// Trashed item names, deleted for good.
    Erase(Vec<String>),
}

/// Running workers and the pipe they report through.
pub struct Jobs {
    read: File,
    write: File,
    buf: Vec<u8>,
    pub running: usize,
}

impl Jobs {
    pub fn new() -> io::Result<Self> {
        let (r, w) = frame::sys::pipe()?;
        frame::sys::set_nonblocking(r.as_fd())?;
        Ok(Jobs {
            read: File::from(r),
            write: File::from(w),
            buf: Vec::new(),
            running: 0,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }

    /// Run `op` on a worker thread.
    pub fn start(&mut self, op: Op) -> io::Result<()> {
        let mut w = self.write.try_clone()?;
        self.running += 1;
        std::thread::spawn(move || {
            let line = run(op).replace('\n', " ");
            let _ = writeln!(w, "{line}");
        });
        Ok(())
    }

    /// Reports that arrived (one per finished job).
    pub fn reports(&mut self) -> Vec<String> {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = self.read.read(&mut chunk) {
            if n == 0 {
                break;
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            out.push(String::from_utf8_lossy(&line[..i]).into_owned());
        }
        self.running = self.running.saturating_sub(out.len());
        out
    }
}

/// Do `op`; a one-line summary for the status bar.
fn run(op: Op) -> String {
    let (verb, results): (&str, Vec<(String, io::Result<()>)>) = match op {
        Op::Copy(src, dir) => ("Copied", each_path(&src, |p| copy_into(p, &dir).map(drop))),
        Op::Move(src, dir) => ("Moved", each_path(&src, |p| move_into(p, &dir).map(drop))),
        Op::Trash(src) => (
            "Moved to Trash:",
            each_path(&src, |p| trash::put(p).map(drop)),
        ),
        Op::Restore(names) => (
            "Restored",
            each_name(&names, |n| trash::restore(n).map(drop)),
        ),
        Op::Erase(names) => ("Deleted", each_name(&names, trash::erase)),
    };
    summary(verb, &results)
}

fn each_path(
    src: &[PathBuf],
    f: impl Fn(&Path) -> io::Result<()>,
) -> Vec<(String, io::Result<()>)> {
    src.iter().map(|p| (name_of(p), f(p))).collect()
}

fn each_name(
    names: &[String],
    f: impl Fn(&str) -> io::Result<()>,
) -> Vec<(String, io::Result<()>)> {
    names.iter().map(|n| (n.clone(), f(n))).collect()
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// "Copied “a.txt”", "Copied 3 items", or the first failure and how many worked.
fn summary(verb: &str, results: &[(String, io::Result<()>)]) -> String {
    let ok = results.iter().filter(|r| r.1.is_ok()).count();
    if let Some((name, Err(e))) = results.iter().find(|r| r.1.is_err()) {
        let rest = if ok > 0 {
            format!(" ({ok} done)")
        } else {
            String::new()
        };
        return format!("Error: \u{201c}{name}\u{201d}: {e}{rest}");
    }
    match results {
        [(name, _)] => format!("{verb} \u{201c}{name}\u{201d}"),
        _ => format!("{verb} {ok} items"),
    }
}

/// A free name in `dir` for `name`: itself, else `stem (copy).ext`, `stem (copy 2).ext` …
pub fn free_name(dir: &Path, name: &OsStr, is_dir: bool) -> OsString {
    if dir.join(name).symlink_metadata().is_err() {
        return name.to_os_string();
    }
    let s = name.to_string_lossy();
    // Folders and dotfiles have no extension to keep.
    let (stem, ext) = match s.rsplit_once('.') {
        Some((st, ex)) if !is_dir && !st.is_empty() => (st.to_string(), format!(".{ex}")),
        _ => (s.to_string(), String::new()),
    };
    (1..)
        .map(|n| match n {
            1 => format!("{stem} (copy){ext}"),
            n => format!("{stem} (copy {n}){ext}"),
        })
        .map(OsString::from)
        .find(|c| dir.join(c).symlink_metadata().is_err())
        .unwrap_or_default()
}

/// Copy `src` into folder `dir` (recursively, links kept as links). Returns the copy.
pub fn copy_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    if dir.starts_with(src) {
        return Err(io::Error::other("can't copy a folder into itself"));
    }
    let meta = src.symlink_metadata()?;
    let name = src.file_name().ok_or(ErrorKind::InvalidInput)?;
    let to = dir.join(free_name(dir, name, meta.is_dir()));
    copy_any(src, &to)?;
    Ok(to)
}

fn copy_any(src: &Path, to: &Path) -> io::Result<()> {
    let meta = src.symlink_metadata()?;
    if meta.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, to)
    } else if meta.is_dir() {
        fs::create_dir(to)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_any(&e.path(), &to.join(e.file_name()))?;
        }
        fs::set_permissions(to, meta.permissions())
    } else {
        fs::copy(src, to).map(drop)
    }
}

/// Move `src` into folder `dir`: a rename, or copy + delete across drives. Moving
/// something into the folder it is already in does nothing.
pub fn move_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    if src.parent() == Some(dir) {
        return Ok(src.to_path_buf());
    }
    if dir.starts_with(src) {
        return Err(io::Error::other("can't move a folder into itself"));
    }
    let meta = src.symlink_metadata()?;
    let name = src.file_name().ok_or(ErrorKind::InvalidInput)?;
    let to = dir.join(free_name(dir, name, meta.is_dir()));
    match fs::rename(src, &to) {
        Err(e) if e.raw_os_error() == Some(EXDEV) => {
            copy_any(src, &to)?;
            if meta.is_dir() {
                fs::remove_dir_all(src)
            } else {
                fs::remove_file(src)
            }?;
        }
        r => r?,
    }
    Ok(to)
}

/// linux errno: cross-device link.
const EXDEV: i32 = 18;

/// Rename `from` (in `dir`) to `to`. Refuses names with `/`, `.` and `..`, empty
/// names, and names already taken.
pub fn rename(dir: &Path, from: &OsStr, to: &str) -> io::Result<()> {
    let to = to.trim();
    if to.is_empty() || to == "." || to == ".." || to.contains('/') {
        return Err(io::Error::other(format!(
            "\u{201c}{to}\u{201d} isn't a valid name"
        )));
    }
    if OsStr::new(to) == from {
        return Ok(());
    }
    let target = dir.join(to);
    if target.symlink_metadata().is_ok() {
        return Err(io::Error::other(format!(
            "\u{201c}{to}\u{201d} exists already"
        )));
    }
    fs::rename(dir.join(from), target)
}

/// Make a new folder in `dir` ("New Folder", "New Folder 2" …). Returns its name.
pub fn new_folder(dir: &Path) -> io::Result<OsString> {
    for n in 1.. {
        let name = if n == 1 {
            "New Folder".to_string()
        } else {
            format!("New Folder {n}")
        };
        match fs::create_dir(dir.join(&name)) {
            Ok(()) => return Ok(name.into()),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("1.. never ends")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-ops-{tag}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copies_get_free_names_and_folders_copy_deep() {
        let d = scratch("copy");
        fs::write(d.join("a.txt"), "1").unwrap();
        fs::create_dir_all(d.join("f/sub")).unwrap();
        fs::write(d.join("f/sub/x"), "2").unwrap();
        let c1 = copy_into(&d.join("a.txt"), &d).unwrap();
        let c2 = copy_into(&d.join("a.txt"), &d).unwrap();
        let f2 = copy_into(&d.join("f"), &d).unwrap();
        let deep = fs::read_to_string(f2.join("sub/x")).unwrap();
        let into_self = copy_into(&d.join("f"), &d.join("f/sub")).is_err();
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(c1.file_name().unwrap(), "a (copy).txt");
        assert_eq!(c2.file_name().unwrap(), "a (copy 2).txt");
        assert_eq!(f2.file_name().unwrap(), "f (copy)");
        assert_eq!(deep, "2");
        assert!(into_self);
    }

    #[test]
    fn move_rename_and_new_folder() {
        let d = scratch("move");
        fs::create_dir_all(d.join("to")).unwrap();
        fs::write(d.join("a"), "1").unwrap();
        let moved = move_into(&d.join("a"), &d.join("to")).unwrap();
        let bad = rename(&d, OsStr::new("to"), "x/y").is_err();
        rename(&d, OsStr::new("to"), "renamed").unwrap();
        let n1 = new_folder(&d).unwrap();
        let n2 = new_folder(&d).unwrap();
        let ok = d.join("renamed/a").exists();
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(moved, d.join("to/a"));
        assert!(bad && ok);
        assert_eq!(
            (n1.to_str(), n2.to_str()),
            (Some("New Folder"), Some("New Folder 2"))
        );
    }

    #[test]
    fn jobs_report_through_the_pipe() {
        let d = scratch("jobs");
        fs::write(d.join("a"), "1").unwrap();
        let mut jobs = Jobs::new().unwrap();
        jobs.start(Op::Copy(vec![d.join("a")], d.clone())).unwrap();
        let mut got = Vec::new();
        for _ in 0..200 {
            got = jobs.reports();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(got, ["Copied \u{201c}a\u{201d}"]);
        assert_eq!(jobs.running, 0);
    }
}
