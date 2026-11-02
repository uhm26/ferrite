//! Archives through bsdtar (libarchive: zip, tar, gz, zst, xz, 7z …): extract into a
//! new folder named after the archive, compress a selection into one file. Run by
//! `ops` workers.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use super::create::free_numbered;
use super::name_of;

/// Archive suffixes, longest first: what `extract` strips to name the folder.
const EXTS: [&str; 10] = [
    ".tar.gz", ".tar.zst", ".tar.xz", ".tar.bz2", ".tgz", ".tar", ".zip", ".7z", ".gz", ".xz",
];

/// The archive formats Compress offers (bsdtar picks the format by suffix).
pub const FORMATS: [&str; 3] = ["zip", "tar.gz", "tar.zst"];

/// Run bsdtar (libarchive: zip, tar, gz, zst, xz, 7z …) with `args`; its error text
/// becomes ours.
fn bsdtar(args: &[&std::ffi::OsStr]) -> io::Result<()> {
    let out = std::process::Command::new("bsdtar").args(args).output()?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(io::Error::other(
        err.lines().next().unwrap_or("bsdtar failed").to_string(),
    ))
}

/// Unpack `archive` into a new folder beside it named after it ("a.zip" → "a", "a 2" …).
pub fn extract(archive: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = name_of(archive);
    let stem = EXTS
        .iter()
        .find_map(|e| name.strip_suffix(e))
        .filter(|s| !s.is_empty())
        .unwrap_or(&name);
    let to = dir.join(free_numbered(dir, stem, ""));
    fs::create_dir(&to)?;
    let r = bsdtar(&[
        "-xf".as_ref(),
        archive.as_os_str(),
        "-C".as_ref(),
        to.as_os_str(),
    ]);
    if r.is_err() {
        let _ = fs::remove_dir_all(&to);
    }
    r.map(|()| to)
}

/// Pack `src` (entries of one folder) into `out`.
pub fn compress(src: &[PathBuf], out: &Path) -> io::Result<()> {
    let dir = src
        .first()
        .and_then(|p| p.parent())
        .ok_or(ErrorKind::InvalidInput)?;
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "-a".as_ref(),
        "-cf".as_ref(),
        out.as_os_str(),
        "-C".as_ref(),
        dir.as_os_str(),
    ];
    args.extend(src.iter().filter_map(|p| p.file_name()));
    bsdtar(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-arch-{tag}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn compress_then_extract_round_trips() {
        let d = scratch("archive");
        fs::create_dir_all(d.join("src/sub")).unwrap();
        fs::write(d.join("src/sub/x.txt"), "hello").unwrap();
        fs::write(d.join("a.md"), "md").unwrap();
        let out = d.join(free_numbered(&d, "pack", "zip"));
        compress(&[d.join("src"), d.join("a.md")], &out).unwrap();
        let to = extract(&out, &d).unwrap();
        let back = fs::read_to_string(to.join("src/sub/x.txt")).unwrap();
        let bad = extract(&d.join("a.md"), &d).is_err();
        let leftover = d.join("a").exists();
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(to.file_name().unwrap(), "pack");
        assert_eq!(back, "hello");
        assert!(bad && !leftover, "a failed extract leaves no folder");
    }
}
