//! Renaming many files at once (F2 on a selection). The typed text is either
//! `old/new`, replacing text in every name (a name can't hold `/`), or a new name
//! where `#` becomes 1, 2, 3… (a number is added when there is no `#`); extensions
//! stay. Nothing is renamed if any new name is taken or comes up twice.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub enum Error {
    /// The text gives no names (empty, or `/new` with nothing to replace).
    Empty,
    /// A new name is already there, or two would get it.
    Taken(String),
    /// A name with a `/`, `.` or `..`.
    Bad(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Empty => write!(f, "type a name, or old/new"),
            Error::Taken(n) => write!(f, "\u{201c}{n}\u{201d} would be there twice"),
            Error::Bad(n) => write!(f, "\u{201c}{n}\u{201d} can't be a file name"),
        }
    }
}

/// The new names for `names` (in that order) from the typed `text`.
pub fn new_names(names: &[String], text: &str) -> Result<Vec<String>, Error> {
    if let Some((old, new)) = text.split_once('/') {
        if old.is_empty() {
            return Err(Error::Empty);
        }
        return Ok(names.iter().map(|n| n.replace(old, new)).collect());
    }
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Empty);
    }
    let digits = names.len().to_string().len();
    Ok(names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let num = format!("{:0digits$}", i + 1);
            let stem = if text.contains('#') {
                text.replace('#', &num)
            } else {
                format!("{text} {num}")
            };
            format!("{stem}{}", ext(n))
        })
        .collect())
}

/// `.jpg` of `a.jpg`; nothing for `.bashrc` or `notes`.
fn ext(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => "",
        Some(i) => &name[i..],
    }
}

/// Rename `paths` (all in one folder) to `names`: every new name is checked first.
/// Returns (old, new) of each one renamed; on a failure halfway, the ones done.
pub fn apply(paths: &[PathBuf], names: &[String]) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut pairs = Vec::new();
    for (p, n) in paths.iter().zip(names) {
        if n.is_empty() || n.contains('/') || n == "." || n == ".." {
            return Err(Error::Bad(n.clone()).to_string());
        }
        let to = p.parent().unwrap_or(Path::new("/")).join(n);
        let taken = to != *p && to.symlink_metadata().is_ok();
        if !seen.insert(to.clone()) || taken {
            return Err(Error::Taken(n.clone()).to_string());
        }
        if to != *p {
            pairs.push((p.clone(), to));
        }
    }
    // shortcut: renames in order, so a swap (a→b, b→a) is refused as taken above
    // instead of going through temporary names.
    let mut done = Vec::new();
    for (from, to) in pairs {
        if let Err(e) = std::fs::rename(&from, &to) {
            return Err(format!("{}: {e} ({} renamed)", from.display(), done.len()));
        }
        done.push((from, to));
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn numbers_keep_extensions_and_replace_works() {
        let n = names(&["a.JPG", "b.png", "notes", ".rc"]);
        assert_eq!(
            new_names(&n, "Trip #").unwrap(),
            ["Trip 1.JPG", "Trip 2.png", "Trip 3", "Trip 4"]
        );
        assert_eq!(new_names(&n, "x").unwrap()[0], "x 1.JPG");
        let many: Vec<String> = (0..12).map(|i| format!("{i}.txt")).collect();
        assert_eq!(new_names(&many, "p#").unwrap()[0], "p01.txt");
        assert_eq!(
            new_names(&names(&["IMG_1.jpg", "IMG_2.jpg"]), "IMG_/day ").unwrap(),
            ["day 1.jpg", "day 2.jpg"]
        );
        assert_eq!(new_names(&n, " "), Err(Error::Empty));
        assert_eq!(new_names(&n, "/x"), Err(Error::Empty));
    }

    #[test]
    fn refuses_taken_names_and_renames_the_rest() {
        let d = std::env::temp_dir().join(format!("files-brename-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for f in ["a.txt", "b.txt", "keep.txt"] {
            std::fs::write(d.join(f), f).unwrap();
        }
        let paths = vec![d.join("a.txt"), d.join("b.txt")];
        let clash = apply(&paths, &names(&["keep.txt", "c.txt"]));
        let twice = apply(&paths, &names(&["c.txt", "c.txt"]));
        let ok = apply(&paths, &names(&["x 1.txt", "x 2.txt"])).unwrap();
        let there = d.join("x 2.txt").exists() && !d.join("a.txt").exists();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(clash.is_err() && twice.is_err());
        assert_eq!(ok.len(), 2);
        assert!(there);
    }
}
