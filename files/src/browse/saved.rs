//! View settings kept from one run to the next: whether hidden files are shown. One
//! empty file, `$XDG_STATE_HOME/files/show-hidden` (default `~/.local/state/…`): there
//! means shown.

use std::io;
use std::path::{Path, PathBuf};

fn file() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("files/show-hidden"))
}

/// Whether the last run left hidden files shown.
pub fn hidden() -> bool {
    file().is_some_and(|f| f.exists())
}

/// Remember `shown` for the next run. Failing only costs that, so it is a note.
pub fn set_hidden(shown: bool) {
    let Some(f) = file() else { return };
    if let Err(e) = write(&f, shown) {
        eprintln!("files: can't remember hidden files in {}: {e}", f.display());
    }
}

fn write(f: &Path, shown: bool) -> io::Result<()> {
    if shown {
        if let Some(dir) = f.parent() {
            std::fs::create_dir_all(dir)?;
        }
        return std::fs::write(f, b"");
    }
    match std::fs::remove_file(f) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_makes_the_file_hidden_removes_it_twice_is_fine() {
        let dir = std::env::temp_dir().join(format!("files-saved-{}", std::process::id()));
        let f = dir.join("files/show-hidden");
        write(&f, true).unwrap();
        assert!(f.exists());
        write(&f, true).unwrap();
        write(&f, false).unwrap();
        assert!(!f.exists());
        write(&f, false).unwrap(); // already gone: not an error
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
