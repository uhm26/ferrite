//! Reading a folder: its entries sorted folders first, then by name, and how their
//! size, type and date read.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use std::{fs, io};

use crate::sys::{self, Local};

#[derive(Clone)]
pub struct Entry {
    /// For display (lossy if the real name isn't UTF-8).
    pub name: String,
    /// The real name, for paths and matching.
    pub os: OsString,
    pub dir: bool,
    /// Bytes; 0 for folders.
    pub size: u64,
    /// How many entries a folder holds (dotfiles only when shown); None for files or
    /// folders we can't read.
    pub items: Option<usize>,
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
            // One read_dir per subfolder, not recursive: what Nemo shows in Size.
            let items = dir.then(|| count(&e.path(), hidden)).flatten();
            Some(Entry {
                name,
                os,
                dir,
                size,
                items,
                modified,
            })
        })
        .collect();
    sort(&mut out, SortBy::Name, false);
    Ok(out)
}

/// Entries in folder `p`, dotfiles only when `hidden`.
fn count(p: &Path, hidden: bool) -> Option<usize> {
    let rd = fs::read_dir(p).ok()?;
    Some(
        rd.filter_map(Result::ok)
            .filter(|e| hidden || !e.file_name().as_encoded_bytes().starts_with(b"."))
            .count(),
    )
}

/// "1 item", "17 items".
pub fn items(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

/// What the list is ordered by (a column).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortBy {
    #[default]
    Name,
    Size,
    Kind,
    Modified,
}

/// Order `entries` by `by` (reversed if `desc`), folders always first. Ties keep name
/// order (the stable base). Folders compare by how many entries they hold.
pub fn sort(entries: &mut [Entry], by: SortBy, desc: bool) {
    use std::cmp::Ordering::Equal;
    entries.sort_by_cached_key(|e| e.name.to_lowercase());
    if by == SortBy::Name && desc {
        entries.reverse();
    }
    entries.sort_by(|a, b| {
        let o = match by {
            SortBy::Name => Equal,
            SortBy::Size if a.dir => a.items.cmp(&b.items),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Kind => kind(a).1.cmp(&kind(b).1),
            SortBy::Modified => a.modified.cmp(&b.modified),
        };
        b.dir.cmp(&a.dir).then(if desc { o.reverse() } else { o })
    });
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
            sec: 0,
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
            items: None,
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
    fn sort_keeps_folders_first_in_every_order() {
        let e = |name: &str, dir, size, modified| Entry {
            name: name.into(),
            os: name.into(),
            dir,
            size,
            items: dir.then_some(size as usize),
            modified,
        };
        let mut v = vec![
            e("b.txt", false, 5, 1),
            e("a.txt", false, 9, 3),
            e("z", true, 1, 2),
            e("y", true, 4, 0),
        ];
        let names = |v: &[Entry]| v.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        sort(&mut v, SortBy::Name, true);
        assert_eq!(names(&v), ["z", "y", "b.txt", "a.txt"]);
        sort(&mut v, SortBy::Size, true);
        assert_eq!(names(&v), ["y", "z", "a.txt", "b.txt"]);
        sort(&mut v, SortBy::Modified, false);
        assert_eq!(names(&v), ["y", "z", "b.txt", "a.txt"]);
    }

    #[test]
    fn folders_count_their_entries() {
        let root = std::env::temp_dir().join(format!("files-count-{}", std::process::id()));
        fs::create_dir_all(root.join("d/sub")).unwrap();
        fs::write(root.join("d/x"), "1").unwrap();
        fs::write(root.join("d/.y"), "1").unwrap();
        let shown = list(&root, false).unwrap()[0].items;
        let all = list(&root, true).unwrap()[0].items;
        fs::remove_dir_all(&root).unwrap();
        assert_eq!((shown, all), (Some(2), Some(3)));
        assert_eq!(items(1), "1 item");
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
