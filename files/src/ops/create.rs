//! Making and naming things inline (they are quick): rename in place, new folder, new
//! file (from a type or a ~/Templates file), numbered names that don't clash.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

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

/// `stem`, `stem 2`, `stem 3` … plus `.ext` when there is one.
fn numbered(stem: &str, ext: &str, n: usize) -> String {
    let base = if n == 1 {
        stem.to_string()
    } else {
        format!("{stem} {n}")
    };
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

/// A name in `dir` not taken yet: `stem.ext`, `stem 2.ext` … (not created).
pub fn free_numbered(dir: &Path, stem: &str, ext: &str) -> String {
    (1..)
        .map(|n| numbered(stem, ext, n))
        .find(|c| dir.join(c).symlink_metadata().is_err())
        .unwrap_or_default()
}

/// Make "Link to <name>" (numbered if taken) in `dir`, pointing at `target`.
/// Returns the link's path.
pub fn link(target: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = target
        .file_name()
        .map_or("/".into(), |n| n.to_string_lossy());
    let at = dir.join(free_numbered(dir, &format!("Link to {name}"), ""));
    std::os::unix::fs::symlink(target, &at)?;
    Ok(at)
}

/// Make a new folder in `dir` ("New Folder", "New Folder 2" …). Returns its name.
pub fn new_folder(dir: &Path) -> io::Result<OsString> {
    for n in 1.. {
        let name = numbered("New Folder", "", n);
        match fs::create_dir(dir.join(&name)) {
            Ok(()) => return Ok(name.into()),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("1.. never ends")
}

/// Make a new file in `dir` named `stem.ext` (numbered if taken) holding `content`,
/// executable when `exec`. Returns its name.
pub fn new_file(
    dir: &Path,
    (stem, ext): (&str, &str),
    content: &[u8],
    exec: bool,
) -> io::Result<OsString> {
    use std::os::unix::fs::PermissionsExt;
    for n in 1.. {
        let name = numbered(stem, ext, n);
        let p = dir.join(&name);
        let mut f = match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        f.write_all(content)?;
        if exec {
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755))?;
        }
        return Ok(name.into());
    }
    unreachable!("1.. never ends")
}

/// Files in `~/Templates` (Nemo's "New Document" source), by name, at most 12.
/// shortcut: the English folder name; read user-dirs.dirs for a localized one.
pub fn templates() -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(crate::view::home().join("Templates")) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v.truncate(12);
    v
}

/// Make a new file in `dir` from template `t` (same name, numbered if taken).
pub fn from_template(dir: &Path, t: &Path) -> io::Result<OsString> {
    use std::os::unix::fs::PermissionsExt;
    let name = t
        .file_name()
        .ok_or(ErrorKind::InvalidInput)?
        .to_string_lossy()
        .into_owned();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, e),
        _ => (name.as_str(), ""),
    };
    let exec = fs::metadata(t)?.permissions().mode() & 0o111 != 0;
    new_file(dir, (stem, ext), &fs::read(t)?, exec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-create-{tag}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rename_rules_and_new_folders() {
        let d = scratch("rename");
        fs::create_dir_all(d.join("to")).unwrap();
        let bad = rename(&d, OsStr::new("to"), "x/y").is_err();
        rename(&d, OsStr::new("to"), "renamed").unwrap();
        let n1 = new_folder(&d).unwrap();
        let n2 = new_folder(&d).unwrap();
        let ok = d.join("renamed").exists();
        fs::remove_dir_all(&d).unwrap();
        assert!(bad && ok);
        assert_eq!(
            (n1.to_str(), n2.to_str()),
            (Some("New Folder"), Some("New Folder 2"))
        );
    }

    #[test]
    fn new_files_number_and_scripts_are_executable() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("newfile");
        let a = new_file(&d, ("New File", "md"), b"", false).unwrap();
        let b = new_file(&d, ("New File", "md"), b"", false).unwrap();
        let s = new_file(&d, ("New File", "sh"), b"#!/bin/sh\n", true).unwrap();
        let t = from_template(&d, &d.join(&s)).unwrap();
        let mode = fs::metadata(d.join(&t)).unwrap().permissions().mode();
        let body = fs::read_to_string(d.join(&t)).unwrap();
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(
            (a.to_str(), b.to_str()),
            (Some("New File.md"), Some("New File 2.md"))
        );
        assert_eq!(t.to_str(), Some("New File 2.sh"));
        assert!(mode & 0o111 != 0 && body == "#!/bin/sh\n");
    }
}
