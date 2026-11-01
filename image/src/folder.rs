//! The images next to the opened one: what Left/Right step through.

use std::path::{Path, PathBuf};

/// Extensions the decoders read (lowercase).
const EXTS: [&str; 3] = ["png", "jpg", "jpeg"];

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The images in `path`'s folder sorted by name (any case), and where `path` is.
/// A folder opens at its first image. A file with another extension is still
/// shown (it may decode, or show why not). Unreadable folder: just `path`.
pub fn siblings(path: &Path) -> (Vec<PathBuf>, usize) {
    let (dir, file) = if path.is_dir() {
        (path, None)
    } else {
        (path.parent().unwrap_or(Path::new(".")), Some(path))
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let mut list: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| is_image(p) && !p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    let Some(file) = file else {
        list.sort_by_cached_key(|p| key(p));
        return (list, 0);
    };
    let file = dir.join(file.file_name().unwrap_or_default());
    if !list.contains(&file) {
        list.push(file.clone());
    }
    list.sort_by_cached_key(|p| key(p));
    let at = list.iter().position(|p| *p == file).unwrap_or(0);
    (list, at)
}

fn key(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_images_sorted_and_finds_the_opened_one() {
        let d = std::env::temp_dir().join("image-test-siblings");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub.png")).unwrap();
        for f in ["b.PNG", "a.jpg", "c.txt", "C.jpeg"] {
            std::fs::write(d.join(f), b"").unwrap();
        }
        let names = |l: &[PathBuf]| -> Vec<String> { l.iter().map(|p| key(p)).collect() };
        let (l, at) = siblings(&d.join("b.PNG"));
        assert_eq!(names(&l), ["a.jpg", "b.png", "c.jpeg"]);
        assert_eq!(at, 1);
        let (l, at) = siblings(&d.join("c.txt"));
        assert_eq!((l.len(), at), (4, 3));
        let (l, at) = siblings(&d);
        assert_eq!((l.len(), at), (3, 0));
        std::fs::remove_dir_all(d).unwrap();
    }
}
