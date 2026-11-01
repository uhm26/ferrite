//! Reading a folder: its entries sorted folders first, then by name, and how their
//! size, type and date read.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use std::{fs, io};

use crate::sys::{self, Local};

pub struct Entry {
    /// For display (lossy if the real name isn't UTF-8).
    pub name: String,
    /// The real name, for paths and matching.
    pub os: OsString,
    pub dir: bool,
    /// Bytes; 0 for folders.
    pub size: u64,
    /// Unix seconds; 0 if unknown.
    pub modified: i64,
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

/// The entries of `path`; dotfiles only when `hidden`.
pub fn list(path: &Path, hidden: bool) -> Result<Vec<Entry>, Error> {
    let err = |e| Error::Read(path.to_path_buf(), e);
    let mut out: Vec<Entry> = fs::read_dir(path)
        .map_err(err)?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let os = e.file_name();
            let name = os.to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            // fs::metadata follows symlinks, so a link to a folder opens like one.
            let meta = fs::metadata(e.path()).ok();
            let dir = meta.as_ref().is_some_and(fs::Metadata::is_dir);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            let size = meta.filter(|_| !dir).map_or(0, |m| m.len());
            Some(Entry {
                name,
                os,
                dir,
                size,
                modified,
            })
        })
        .collect();
    out.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
    Ok(out)
}

/// A short size: `512 B`, `4.2 KB`, `13 MB`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
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

/// What an entry is, from its extension: picks the icon and the Type column.
/// shortcut: a small extension table, not shared-mime-info; read the MIME database
/// if names show up wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Image,
    Audio,
    Video,
    Text,
    Code,
    Archive,
    Other,
}

/// Extension → (kind, Type column text).
const TYPES: &[(&str, Kind, &str)] = &[
    ("png", Kind::Image, "PNG image"),
    ("jpg", Kind::Image, "JPEG image"),
    ("jpeg", Kind::Image, "JPEG image"),
    ("gif", Kind::Image, "GIF image"),
    ("webp", Kind::Image, "WebP image"),
    ("svg", Kind::Image, "SVG image"),
    ("mp3", Kind::Audio, "MP3 audio"),
    ("flac", Kind::Audio, "FLAC audio"),
    ("ogg", Kind::Audio, "Ogg audio"),
    ("opus", Kind::Audio, "Opus audio"),
    ("wav", Kind::Audio, "WAV audio"),
    ("mp4", Kind::Video, "MP4 video"),
    ("mkv", Kind::Video, "MKV video"),
    ("webm", Kind::Video, "WebM video"),
    ("mov", Kind::Video, "QuickTime video"),
    ("md", Kind::Text, "Markdown"),
    ("txt", Kind::Text, "Text"),
    ("pdf", Kind::Text, "PDF document"),
    ("log", Kind::Text, "Log"),
    ("rs", Kind::Code, "Rust"),
    ("toml", Kind::Code, "TOML"),
    ("json", Kind::Code, "JSON"),
    ("yaml", Kind::Code, "YAML"),
    ("yml", Kind::Code, "YAML"),
    ("lua", Kind::Code, "Lua"),
    ("sh", Kind::Code, "Shell script"),
    ("c", Kind::Code, "C"),
    ("h", Kind::Code, "C header"),
    ("cpp", Kind::Code, "C++"),
    ("js", Kind::Code, "JavaScript"),
    ("ts", Kind::Code, "TypeScript"),
    ("css", Kind::Code, "CSS"),
    ("html", Kind::Code, "HTML"),
    ("lock", Kind::Code, "Lock file"),
    ("zip", Kind::Archive, "ZIP archive"),
    ("tar", Kind::Archive, "Tar archive"),
    ("gz", Kind::Archive, "Gzip archive"),
    ("xz", Kind::Archive, "XZ archive"),
    ("zst", Kind::Archive, "Zstd archive"),
    ("7z", Kind::Archive, "7z archive"),
];

/// The entry's kind and its Type column text.
pub fn kind(e: &Entry) -> (Kind, String) {
    if e.dir {
        return (Kind::Folder, "Folder".into());
    }
    let ext = Path::new(&e.name)
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase());
    let Some(ext) = ext else {
        return (Kind::Other, "File".into());
    };
    match TYPES.iter().find(|t| t.0 == ext) {
        Some(&(_, k, name)) => (k, name.into()),
        None => (Kind::Other, format!("{} file", ext.to_uppercase())),
    }
}

/// The date column: `17:37` today, `1 Nov` this year, else `1 Nov 2024`.
pub fn date(t: Local, now: Local) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = MONTHS[usize::from(t.month.min(11))];
    if (t.year, t.month, t.day) == (now.year, now.month, now.day) {
        format!("{:02}:{:02}", t.hour, t.min)
    } else if t.year == now.year {
        format!("{} {mon}", t.day)
    } else {
        format!("{} {mon} {}", t.day, t.year)
    }
}

/// [`date`] for unix seconds against the clock.
pub fn date_of(secs: i64, now: Local) -> String {
    date(sys::local(secs), now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(4300), "4.2 KB");
        assert_eq!(human(13 * 1024 * 1024), "13 MB");
    }

    #[test]
    fn dates_shorten_near_now() {
        let at = |year, month, day, hour| Local {
            year,
            month,
            day,
            hour,
            min: 7,
        };
        let now = at(2026, 10, 1, 17);
        assert_eq!(date(at(2026, 10, 1, 9), now), "09:07");
        assert_eq!(date(at(2026, 9, 30, 9), now), "30 Oct");
        assert_eq!(date(at(2024, 0, 5, 9), now), "5 Jan 2024");
    }

    #[test]
    fn kinds() {
        let e = |name: &str, dir| Entry {
            name: name.into(),
            os: name.into(),
            dir,
            size: 0,
            modified: 0,
        };
        assert_eq!(kind(&e("src", true)), (Kind::Folder, "Folder".into()));
        assert_eq!(kind(&e("a.RS", false)), (Kind::Code, "Rust".into()));
        assert_eq!(kind(&e("Makefile", false)), (Kind::Other, "File".into()));
        assert_eq!(
            kind(&e("x.weird", false)),
            (Kind::Other, "WEIRD file".into())
        );
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
        let names: Vec<_> = list(&root, false)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(names, ["A", "b", "a.txt", "C.txt"]);
    }
}
