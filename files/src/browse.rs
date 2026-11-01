//! Where we are: the current folder, its entries, the selected one, back/forward
//! history, and moving between folders. No drawing here.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Command;

use crate::dir::{self, Entry, SortBy};
use crate::sys;

pub struct Browser {
    pub cwd: PathBuf,
    /// What the list shows: `all` through the filter.
    pub entries: Vec<Entry>,
    /// Every entry of the folder, sorted.
    all: Vec<Entry>,
    /// Show only names containing this (Ctrl+F); cleared on another folder.
    pub filter: String,
    pub sel: usize,
    /// Free and total bytes on the current folder's filesystem, if known.
    pub disk: Option<(u64, u64)>,
    /// The last thing that went wrong, shown until the next move.
    pub error: Option<String>,
    /// Show dotfiles (Ctrl+H).
    pub hidden: bool,
    /// Bumped on every successful load: views cache per-folder measurements by it.
    pub generation: u64,
    /// Column the entries are ordered by, and whether descending (folders first always).
    pub sort: (SortBy, bool),
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl Browser {
    /// Open on `cwd`; if it can't be read, on home, then `/` (the error stays shown).
    pub fn new(cwd: PathBuf) -> Self {
        let mut b = Browser {
            cwd: PathBuf::new(),
            entries: Vec::new(),
            all: Vec::new(),
            filter: String::new(),
            sel: 0,
            disk: None,
            error: None,
            hidden: false,
            generation: 0,
            sort: (SortBy::Name, false),
            back: Vec::new(),
            forward: Vec::new(),
        };
        if !b.load(cwd, None) {
            let why = b.error.take();
            if !b.load(crate::view::home(), None) {
                b.load(PathBuf::from("/"), None);
            }
            b.error = why;
        }
        b
    }

    /// Show folder `path`, selecting the entry named `select` if it is there. On
    /// failure stay put. Another folder drops the filter.
    fn load(&mut self, path: PathBuf, select: Option<&OsStr>) -> bool {
        match dir::list(&path, self.hidden) {
            Ok(mut entries) => {
                dir::sort(&mut entries, self.sort.0, self.sort.1);
                if path != self.cwd {
                    self.filter.clear();
                }
                self.disk = sys::disk(&path).ok();
                (self.cwd, self.all, self.error) = (path, entries, None);
                self.show(select.map(OsStr::to_os_string));
                self.generation += 1;
                true
            }
            Err(e) => {
                self.error = Some(e.to_string());
                false
            }
        }
    }

    /// Fill `entries` from `all` through the filter, selecting `keep` if shown.
    fn show(&mut self, keep: Option<OsString>) {
        let f = self.filter.to_lowercase();
        self.entries = self
            .all
            .iter()
            .filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f))
            .cloned()
            .collect();
        self.sel = keep
            .and_then(|k| self.entries.iter().position(|e| e.os == k))
            .unwrap_or(0);
    }

    fn selected_name(&self) -> Option<OsString> {
        self.entries.get(self.sel).map(|e| e.os.clone())
    }

    /// Order by `by`; the same column again flips the direction. The selected entry
    /// stays selected.
    pub fn sort_by(&mut self, by: SortBy) {
        self.sort = (by, self.sort.0 == by && !self.sort.1);
        dir::sort(&mut self.all, self.sort.0, self.sort.1);
        self.show(self.selected_name());
    }

    /// Show only entries whose name contains `text` (any case); empty shows all.
    pub fn set_filter(&mut self, text: &str) {
        self.filter = text.to_string();
        self.show(self.selected_name());
        self.generation += 1;
    }

    /// Read the folder again and select `name` (after a rename or new folder).
    pub fn reload_select(&mut self, name: &OsStr) {
        self.load(self.cwd.clone(), Some(name));
    }

    /// Show or hide dotfiles, keeping the selected entry selected.
    pub fn toggle_hidden(&mut self) {
        self.hidden = !self.hidden;
        self.reload();
    }

    /// Read the folder again (it changed on disk), keeping the selected entry
    /// selected. If the folder itself is gone, step up to the nearest one left.
    pub fn reload(&mut self) {
        let keep = self.entries.get(self.sel).map(|e| e.os.clone());
        if self.load(self.cwd.clone(), keep.as_deref()) {
            return;
        }
        let gone = self.cwd.clone();
        let mut at = gone.clone();
        while at.pop() {
            let from = gone.strip_prefix(&at).ok().and_then(|r| r.iter().next());
            if self.load(at.clone(), from) {
                self.error = Some(format!("{} is gone", gone.display()));
                return;
            }
        }
    }

    /// Go to `path` as a new step: back returns here, forward is forgotten.
    pub fn go(&mut self, path: PathBuf, select: Option<&OsStr>) {
        let old = self.cwd.clone();
        if path != old && self.load(path, select) {
            self.back.push(old);
            self.forward.clear();
        }
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// One step back (`back`) or forward in history. A folder that is gone drops out.
    pub fn travel(&mut self, back: bool) {
        let to = if back {
            self.back.pop()
        } else {
            self.forward.pop()
        };
        let Some(to) = to else {
            return;
        };
        let old = self.cwd.clone();
        // Select the folder we came from when it is in the target (going back up).
        if self.load(to, old.file_name()) {
            if back {
                &mut self.forward
            } else {
                &mut self.back
            }
            .push(old);
        }
    }

    /// Enter the selected folder, or hand a file to its default app.
    pub fn open(&mut self) {
        let Some(e) = self.entries.get(self.sel) else {
            return;
        };
        // The real name, not the display one: non-UTF-8 names must still open.
        let path = self.cwd.join(&e.os);
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
        let from = self.cwd.file_name().map(OsStr::to_os_string);
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

    fn tree(tag: &str, dirs: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("files-{tag}-{}", std::process::id()));
        for d in dirs {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        root
    }

    #[test]
    fn into_and_back_up_keeps_place() {
        let root = tree("browse", &["a", "b/inner"]);
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
        let root = tree("up", &["a", "b", "c"]);
        let mut b = Browser::new(root.join("c"));
        b.up();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(b.entries[b.sel].name, "c");
    }

    #[test]
    fn hidden_toggle_keeps_selection() {
        let root = tree("hidden", &[".git", "b", "c"]);
        let mut b = Browser::new(root.clone());
        b.step(1);
        b.toggle_hidden();
        let shown = (b.entries.len(), b.entries[b.sel].name.clone());
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(shown, (3, "c".into()));
    }

    #[test]
    fn reload_sees_changes_and_survives_a_removed_folder() {
        let root = tree("reload", &["a/inner"]);
        let mut b = Browser::new(root.join("a"));
        fs::write(root.join("a/new.txt"), "x").unwrap();
        b.reload();
        let after_write = b.entries.len();
        fs::remove_dir_all(root.join("a")).unwrap();
        b.reload();
        let landed = b.cwd.clone();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(after_write, 2);
        assert_eq!(landed, root);
        assert!(b.error.is_some());
    }

    #[test]
    fn non_utf8_folders_open_and_reselect() {
        use std::os::unix::ffi::OsStrExt;
        let root = tree("bytes", &[]);
        let odd = OsStr::from_bytes(b"caf\xe9");
        fs::create_dir_all(root.join(odd).join("in")).unwrap();
        let mut b = Browser::new(root.clone());
        b.open();
        let inside = b.cwd.clone();
        b.up();
        let back_on = b.entries[b.sel].os.clone();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(inside, root.join(odd));
        assert_eq!(back_on, odd);
    }

    #[test]
    fn unreadable_start_falls_back_and_says_why() {
        let b = Browser::new(PathBuf::from("/no/such/folder"));
        assert!(!b.cwd.as_os_str().is_empty());
        assert!(b.error.is_some());
    }

    #[test]
    fn back_and_forward() {
        let root = tree("hist", &["a/deep", "b"]);
        let mut b = Browser::new(root.clone());
        b.go(root.join("a"), None);
        b.go(root.join("a/deep"), None);
        b.travel(true);
        let after_back = (b.cwd.clone(), b.entries[b.sel].name.clone());
        b.travel(false);
        let after_fwd = b.cwd.clone();
        b.travel(true);
        b.go(root.join("b"), None);
        let fwd_cleared = !b.can_forward();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(after_back, (root.join("a"), "deep".into()));
        assert_eq!(after_fwd, root.join("a/deep"));
        assert!(fwd_cleared);
    }
}
