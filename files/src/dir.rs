//! Reading a directory: its entries sorted folders first, then by name.

use std::fmt;
use std::path::{Path, PathBuf};
use std::{fs, io};

pub struct Entry {
    pub name: String,
    pub dir: bool,
    /// Bytes; 0 for folders.
    pub size: u64,
}

#[derive(Debug)]
pub enum Error {
    Read(PathBuf, io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Read(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

/// The entries of `path`, dotfiles hidden.
pub fn list(path: &Path) -> Result<Vec<Entry>, Error> {
    let err = |e| Error::Read(path.to_path_buf(), e);
    let mut out: Vec<Entry> = fs::read_dir(path)
        .map_err(err)?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // fs::metadata follows symlinks, so a link to a folder opens like one.
            let meta = fs::metadata(e.path()).ok();
            let dir = meta.as_ref().is_some_and(fs::Metadata::is_dir);
            let size = meta.filter(|_| !dir).map_or(0, |m| m.len());
            (!name.starts_with('.')).then_some(Entry { name, dir, size })
        })
        .collect();
    out.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
    Ok(out)
}

/// A short size: `512 B`, `4.2 K`, `13 M`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 || v >= 10.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(4300), "4.2 K");
        assert_eq!(human(13 * 1024 * 1024), "13 M");
    }

    #[test]
    fn folders_first_then_name_no_dotfiles() {
        let root = std::env::temp_dir().join(format!("files-test-{}", std::process::id()));
        for d in ["b", "A"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in ["a.txt", "C.txt", ".hidden"] {
            fs::write(root.join(f), "x").unwrap();
        }
        let names: Vec<_> = list(&root).unwrap().into_iter().map(|e| e.name).collect();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(names, ["A", "b", "a.txt", "C.txt"]);
    }
}
