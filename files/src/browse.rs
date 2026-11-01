//! Where we are: the current folder, its entries, the selected one, and moving
//! between folders. No drawing here.

use std::path::PathBuf;
use std::process::Command;

use crate::dir::{self, Entry};

pub struct Browser {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub sel: usize,
    /// The last thing that went wrong, shown until the next move.
    pub error: Option<String>,
}

impl Browser {
    pub fn new(cwd: PathBuf) -> Self {
        let mut b = Browser {
            cwd: PathBuf::new(),
            entries: Vec::new(),
            sel: 0,
            error: None,
        };
        b.go(cwd, None);
        b
    }

    /// Show folder `path`, selecting `select` if it is there. On failure stay put.
    pub fn go(&mut self, path: PathBuf, select: Option<&str>) {
        match dir::list(&path) {
            Ok(entries) => {
                self.sel = select
                    .and_then(|s| entries.iter().position(|e| e.name == s))
                    .unwrap_or(0);
                (self.cwd, self.entries, self.error) = (path, entries, None);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Enter the selected folder, or hand a file to its default app.
    pub fn open(&mut self) {
        let Some(e) = self.entries.get(self.sel) else {
            return;
        };
        let path = self.cwd.join(&e.name);
        if e.dir {
            self.go(path, None);
            return;
        }
        match Command::new("xdg-open").arg(&path).spawn() {
            // Reap it off-thread so no zombie stays behind.
            Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
            Err(e) => self.error = Some(format!("xdg-open: {e}")),
        }
    }

    /// Go to the parent, keeping the folder we came from selected.
    pub fn up(&mut self) {
        let Some(parent) = self.cwd.parent().map(PathBuf::from) else {
            return;
        };
        let from = self
            .cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        self.go(parent, from.as_deref());
    }

    /// Move the selection by `d` rows, stopping at the ends.
    pub fn step(&mut self, d: isize) {
        let last = self.entries.len().saturating_sub(1);
        self.sel = self.sel.saturating_add_signed(d).min(last);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn into_and_back_up_keeps_place() {
        let root = std::env::temp_dir().join(format!("files-browse-{}", std::process::id()));
        for d in ["a", "b/inner"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut b = Browser::new(root.clone());
        b.step(1);
        b.open();
        let inside = b.cwd.clone();
        b.up();
        b.step(-5);
        let top = b.sel;
        b.go(root.join("missing"), None);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(inside, root.join("b"));
        assert_eq!(top, 0);
        assert_eq!(b.cwd, root, "a failed go stays put");
        assert!(b.error.is_some());
    }

    #[test]
    fn up_selects_the_folder_we_left() {
        let root = std::env::temp_dir().join(format!("files-up-{}", std::process::id()));
        for d in ["a", "b", "c"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut b = Browser::new(root.join("c"));
        b.up();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(b.entries[b.sel].name, "c");
    }
}
