//! Search: names containing some text (any case) in a folder and all its subfolders.
//! A worker thread walks them, shallow folders first, and the results stream in while
//! it runs. It stays on the folder's filesystem (no /proc from /) and doesn't follow
//! links into other folders. Dropping a [`Search`] stops its walk.

use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use crate::dir::{self, Entry};

/// Results kept at most; the walk stops there.
pub const MAX_RESULTS: usize = 5000;
/// How often found entries are handed over while walking.
const BATCH: Duration = Duration::from_millis(50);

pub struct Search {
    pub text: String,
    /// Not a search: the recent files (recent.rs), all there at once.
    pub recent: bool,
    stop: Arc<AtomicBool>,
    rx: Receiver<Vec<Entry>>,
    /// The walk is over (everything found was taken).
    pub done: bool,
}

impl Search {
    /// Start looking for `text` under `root`; dotfiles only when `hidden`. Results'
    /// names are paths relative to `root` ("src/app.rs").
    pub fn start(root: PathBuf, text: &str, hidden: bool) -> Search {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let needle = text.to_lowercase();
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || walk(&root, (&needle, hidden), &flag, &tx));
        Search {
            text: text.to_string(),
            recent: false,
            stop,
            rx,
            done: false,
        }
    }

    /// The recent files' listing: nothing to walk, done at once.
    pub fn recent() -> Search {
        let (_, rx) = channel();
        Search {
            text: "Recent".into(),
            recent: true,
            stop: Arc::default(),
            rx,
            done: true,
        }
    }

    /// Results found since last asked (sets `done` at the end of the walk).
    pub fn take(&mut self) -> Vec<Entry> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(batch) => out.extend(batch),
                Err(TryRecvError::Empty) => return out,
                Err(TryRecvError::Disconnected) => {
                    self.done = true;
                    return out;
                }
            }
        }
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Walk `root` breadth first, sending matches in batches until done, stopped, or
/// [`MAX_RESULTS`].
fn walk(root: &Path, (needle, hidden): (&str, bool), stop: &AtomicBool, tx: &Sender<Vec<Entry>>) {
    let dev = fs::metadata(root).map_or(0, |m| m.dev());
    let mut todo = VecDeque::from([PathBuf::new()]);
    let (mut batch, mut found, mut sent) = (Vec::new(), 0, Instant::now());
    while let Some(rel) = todo.pop_front() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Ok(rd) = fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for e in rd.flatten() {
            let os = e.file_name();
            let name = os.to_string_lossy();
            if !hidden && name.starts_with('.') {
                continue;
            }
            let rel = rel.join(&os);
            // file_type doesn't follow links: a link to a folder isn't walked into.
            let into = e.file_type().is_ok_and(|t| t.is_dir())
                && e.metadata().is_ok_and(|m| m.dev() == dev);
            if name.to_lowercase().contains(needle) {
                let shown = rel.to_string_lossy().into_owned();
                let path = root.join(&rel);
                batch.push(dir::entry(
                    &path,
                    (rel.clone().into_os_string(), shown),
                    hidden,
                ));
                found += 1;
            }
            if into {
                todo.push_back(rel);
            }
        }
        if found >= MAX_RESULTS {
            break;
        }
        if !batch.is_empty() && sent.elapsed() >= BATCH {
            if tx.send(std::mem::take(&mut batch)).is_err() {
                return;
            }
            sent = Instant::now();
        }
    }
    let _ = tx.send(batch);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_names_in_subfolders_any_case_and_skips_dotfiles() {
        let root = std::env::temp_dir().join(format!("files-search-{}", std::process::id()));
        for d in ["a/deep/Notes", "b", ".hidden/notes"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        fs::write(root.join("b/notes.txt"), "x").unwrap();
        fs::write(root.join("b/other.txt"), "x").unwrap();
        let mut s = Search::start(root.clone(), "NOTES", false);
        let mut found = Vec::new();
        let start = Instant::now();
        while !s.done {
            assert!(start.elapsed().as_secs() < 5, "walk stuck");
            found.extend(s.take());
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut names: Vec<String> = found.iter().map(|e| e.name.clone()).collect();
        names.sort();
        let dir_flag = found
            .iter()
            .find(|e| e.name == "a/deep/Notes")
            .map(|e| e.dir);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(names, ["a/deep/Notes", "b/notes.txt"]);
        assert_eq!(dir_flag, Some(true));
    }
}
