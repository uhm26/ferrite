//! Copying and moving: recursive, links kept as links, in 1 MiB chunks so a running
//! job shows progress and can be cancelled (a cancelled file is removed, never left
//! half-written). A name already taken in the target folder is settled by
//! [`Conflict`]: keep both (`a (copy).txt`), skip, or replace — the old one goes to
//! the Trash, so a wrong "Replace" can be undone by hand.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

use super::trash;

/// linux errno: cross-device link.
const EXDEV: i32 = 18;
const CHUNK: usize = 1 << 20;

/// What to do when the target folder already has an entry of that name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    KeepBoth,
    Skip,
    /// The existing one goes to the Trash first.
    Replace,
}

/// A job's progress, shared with the window: bytes done of the total, and a flag the
/// window sets to cancel.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    fn add(&self, n: u64) -> io::Result<()> {
        self.done.fetch_add(n, Relaxed);
        if self.cancel.load(Relaxed) {
            return Err(io::Error::new(ErrorKind::Interrupted, "cancelled"));
        }
        Ok(())
    }
}

/// Bytes and files under `p` (links not followed; unreadable parts count as empty).
pub fn measure(p: &Path) -> (u64, u64) {
    let Ok(m) = p.symlink_metadata() else {
        return (0, 0);
    };
    if !m.is_dir() {
        return (m.len(), 1);
    }
    let Ok(rd) = fs::read_dir(p) else {
        return (0, 0);
    };
    rd.filter_map(Result::ok)
        .map(|e| measure(&e.path()))
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
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

/// Names of `src` already taken in `dir` (for asking before a paste or drop). Moving
/// something into the folder it is in doesn't count.
pub fn clashes(src: &[PathBuf], dir: &Path, moving: bool) -> usize {
    src.iter()
        .filter(|p| !(moving && p.parent() == Some(dir)))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| dir.join(n).symlink_metadata().is_ok())
        })
        .count()
}

/// Where `src` lands in `dir`; None: skip it.
fn target(src: &Path, dir: &Path, c: Conflict) -> io::Result<Option<PathBuf>> {
    if dir.starts_with(src) {
        return Err(io::Error::other("can't put a folder into itself"));
    }
    let name = src.file_name().ok_or(ErrorKind::InvalidInput)?;
    let to = dir.join(name);
    if to.symlink_metadata().is_err() {
        return Ok(Some(to));
    }
    match c {
        Conflict::Skip => Ok(None),
        Conflict::KeepBoth => Ok(Some(dir.join(free_name(dir, name, src.is_dir())))),
        Conflict::Replace if to == src => Ok(None),
        Conflict::Replace => trash::put(&to).map(|_| Some(to)),
    }
}

/// Copy `src` into folder `dir`. Returns the copy, or None if skipped.
pub fn copy_into(
    src: &Path,
    dir: &Path,
    c: Conflict,
    prog: &Progress,
) -> io::Result<Option<PathBuf>> {
    // Copying beside itself always keeps both (Nemo's "a (copy)").
    let c = if src.parent() == Some(dir) {
        Conflict::KeepBoth
    } else {
        c
    };
    let Some(to) = target(src, dir, c)? else {
        return Ok(None);
    };
    copy_any(src, &to, prog).inspect_err(|_| remove_any(&to))?;
    Ok(Some(to))
}

/// Move `src` into folder `dir`: a rename, or copy + delete across drives. Moving into
/// the folder it is already in does nothing (None).
pub fn move_into(
    src: &Path,
    dir: &Path,
    c: Conflict,
    prog: &Progress,
) -> io::Result<Option<PathBuf>> {
    if src.parent() == Some(dir) {
        return Ok(None);
    }
    let Some(to) = target(src, dir, c)? else {
        return Ok(None);
    };
    match fs::rename(src, &to) {
        Err(e) if e.raw_os_error() == Some(EXDEV) => {
            copy_any(src, &to, prog).inspect_err(|_| remove_any(&to))?;
            remove_tree(src)?;
        }
        r => {
            r?;
            prog.add(measure(&to).0)?;
        }
    }
    Ok(Some(to))
}

/// Put `to` back at `from` (undoing a move).
pub fn move_back(to: &Path, from: &Path) -> io::Result<()> {
    if from.symlink_metadata().is_ok() {
        return Err(io::Error::other(format!("{} exists again", from.display())));
    }
    match fs::rename(to, from) {
        Err(e) if e.raw_os_error() == Some(EXDEV) => {
            copy_any(to, from, &Progress::default())?;
            remove_tree(to)
        }
        r => r,
    }
}

fn copy_any(src: &Path, to: &Path, prog: &Progress) -> io::Result<()> {
    let meta = src.symlink_metadata()?;
    if meta.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, to)
    } else if meta.is_dir() {
        fs::create_dir(to)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_any(&e.path(), &to.join(e.file_name()), prog)?;
        }
        fs::set_permissions(to, meta.permissions())
    } else {
        copy_file(src, to, prog)?;
        fs::set_permissions(to, meta.permissions())
    }
}

/// One file in chunks: progress per chunk, cancel between chunks.
fn copy_file(src: &Path, to: &Path, prog: &Progress) -> io::Result<()> {
    let mut from = File::open(src)?;
    let mut out = OpenOptions::new().write(true).create_new(true).open(to)?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = from.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(&buf[..n])?;
        prog.add(n as u64)?;
    }
}

fn remove_tree(p: &Path) -> io::Result<()> {
    if p.symlink_metadata()?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

/// Best-effort cleanup of a half-made copy.
fn remove_any(p: &Path) {
    let _ = remove_tree(p);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-copy-{tag}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copies_get_free_names_and_folders_copy_deep() {
        let d = scratch("copy");
        let p = Progress::default();
        fs::write(d.join("a.txt"), "1").unwrap();
        fs::create_dir_all(d.join("f/sub")).unwrap();
        fs::write(d.join("f/sub/x"), "2").unwrap();
        let c1 = copy_into(&d.join("a.txt"), &d, Conflict::Skip, &p)
            .unwrap()
            .unwrap();
        let c2 = copy_into(&d.join("a.txt"), &d, Conflict::Skip, &p)
            .unwrap()
            .unwrap();
        let f2 = copy_into(&d.join("f"), &d, Conflict::KeepBoth, &p)
            .unwrap()
            .unwrap();
        let deep = fs::read_to_string(f2.join("sub/x")).unwrap();
        let into_self = copy_into(&d.join("f"), &d.join("f/sub"), Conflict::KeepBoth, &p).is_err();
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(c1.file_name().unwrap(), "a (copy).txt");
        assert_eq!(c2.file_name().unwrap(), "a (copy 2).txt");
        assert_eq!(f2.file_name().unwrap(), "f (copy)");
        assert_eq!(deep, "2");
        assert!(into_self);
    }

    #[test]
    fn conflicts_skip_keep_both_and_cancel() {
        let d = scratch("conflict");
        fs::create_dir_all(d.join("to")).unwrap();
        fs::write(d.join("a"), "new").unwrap();
        fs::write(d.join("to/a"), "old").unwrap();
        let p = Progress::default();
        assert_eq!(clashes(&[d.join("a")], &d.join("to"), false), 1);
        let skipped = copy_into(&d.join("a"), &d.join("to"), Conflict::Skip, &p).unwrap();
        let both = copy_into(&d.join("a"), &d.join("to"), Conflict::KeepBoth, &p).unwrap();
        let cancelled = Progress::default();
        cancelled.cancel.store(true, Relaxed);
        let stopped = copy_into(&d.join("a"), &d.join("to"), Conflict::KeepBoth, &cancelled);
        let left = fs::read_dir(d.join("to")).unwrap().count();
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(skipped, None);
        assert_eq!(both.unwrap().file_name().unwrap(), "a (copy)");
        assert!(stopped.is_err());
        assert_eq!(left, 2, "the cancelled copy left nothing behind");
    }

    #[test]
    fn move_and_move_back() {
        let d = scratch("move");
        fs::create_dir_all(d.join("to")).unwrap();
        fs::write(d.join("a"), "1").unwrap();
        let p = Progress::default();
        let moved = move_into(&d.join("a"), &d.join("to"), Conflict::KeepBoth, &p)
            .unwrap()
            .unwrap();
        let same = move_into(&moved, &d.join("to"), Conflict::KeepBoth, &p).unwrap();
        move_back(&moved, &d.join("a")).unwrap();
        let back = d.join("a").exists() && !moved.exists();
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(moved, d.join("to/a"));
        assert_eq!(same, None, "moving into its own folder does nothing");
        assert!(back);
        assert_eq!(p.done.load(Relaxed), 1);
    }
}
