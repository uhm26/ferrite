//! Bookmarked folders, in GTK's file (`~/.config/gtk-3.0/bookmarks`) so Nemo, GTK
//! file pickers and we share them. One `file://` URI per line, optionally a label
//! after a space. Lines we don't understand (other schemes) are kept as they are.

use std::path::{Path, PathBuf};
use std::{fs, io};

use crate::uri;

fn file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::view::home().join(".config"));
    config.join("gtk-3.0/bookmarks")
}

/// The bookmarks: label and folder (label = the folder name when the line has none).
pub fn load() -> Vec<(String, PathBuf)> {
    let text = fs::read_to_string(file()).unwrap_or_default();
    parse(&text)
}

fn parse(text: &str) -> Vec<(String, PathBuf)> {
    text.lines()
        .filter_map(|line| {
            let (u, label) = line.split_once(' ').unwrap_or((line, ""));
            let p = uri::from_file_uri(u)?;
            let name = p
                .file_name()
                .map_or("/".into(), |n| n.to_string_lossy().into_owned());
            Some((
                if label.is_empty() {
                    name
                } else {
                    label.to_string()
                },
                p,
            ))
        })
        .collect()
}

pub fn has(p: &Path) -> bool {
    load().iter().any(|(_, b)| b == p)
}

/// Add `p` if missing, else remove it. Returns whether it is bookmarked now.
pub fn toggle(p: &Path) -> io::Result<bool> {
    let f = file();
    let text = fs::read_to_string(&f).unwrap_or_default();
    let (new, now) = toggled(&text, p);
    if let Some(dir) = f.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&f, new)?;
    Ok(now)
}

/// The file text with `p` added or removed, and whether it is in it now.
fn toggled(text: &str, p: &Path) -> (String, bool) {
    let mine = |line: &str| {
        let u = line.split_once(' ').map_or(line, |(u, _)| u);
        uri::from_file_uri(u).as_deref() == Some(p)
    };
    if text.lines().any(mine) {
        let kept: Vec<&str> = text.lines().filter(|l| !mine(l)).collect();
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        (out, false)
    } else {
        let mut out = text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&uri::to_file_uri(p));
        out.push('\n');
        (out, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_toggle_keep_other_lines() {
        let text = "file:///home/g/projects\nfile:///mnt/x%20y Music\nsftp://host/\n";
        let b = parse(text);
        assert_eq!(b[0], ("projects".into(), PathBuf::from("/home/g/projects")));
        assert_eq!(b[1], ("Music".into(), PathBuf::from("/mnt/x y")));
        assert_eq!(b.len(), 2);
        let (removed, now) = toggled(text, Path::new("/home/g/projects"));
        assert!(!now);
        assert_eq!(removed, "file:///mnt/x%20y Music\nsftp://host/\n");
        let (added, now) = toggled(&removed, Path::new("/tmp/a b"));
        assert!(now);
        assert!(added.ends_with("sftp://host/\nfile:///tmp/a%20b\n"));
    }
}
