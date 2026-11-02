//! File operations. Long ones run on worker threads so the window stays responsive:
//! each sends a [`Report`] (a status line and how to undo it, or a folder's size) on
//! a channel and wakes the event loop through a pipe it polls; copies and moves show
//! progress and can be cancelled. Quick ones (rename, new folder or file) are in
//! [`create`] and run inline.

pub mod archive;
pub mod copy;
pub mod create;
pub mod rename;
pub mod trash;

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{Receiver, Sender, channel};

use copy::{Conflict, Progress};

/// What a worker does.
pub enum Op {
    /// Copy these into the folder.
    Copy(Vec<PathBuf>, PathBuf, Conflict),
    /// Move these into the folder.
    Move(Vec<PathBuf>, PathBuf, Conflict),
    Trash(Vec<PathBuf>),
    /// Trashed item names back to where they came from.
    Restore(Vec<String>),
    /// Trashed item names, deleted for good.
    Erase(Vec<String>),
    /// Everything in the Trash, deleted for good.
    EmptyTrash,
    /// Unpack the archive into a new folder named after it, in the folder.
    Extract(PathBuf, PathBuf),
    /// Pack these (all in one folder) into the archive path; its suffix picks the format.
    Compress(Vec<PathBuf>, PathBuf),
    /// Undo a move: each (where it was, where it is) back.
    MoveBack(Vec<(PathBuf, PathBuf)>),
    /// Add up a folder's size (for the info pane).
    Measure(PathBuf),
}

/// How to take a finished job back (Ctrl+Z).
#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    /// Moved: (where it was, where it is now).
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Made: copies, a new folder or file, an archive, extracted or restored items.
    /// Undo puts them in the Trash.
    Made(Vec<PathBuf>),
    /// Put in the Trash under these names: undo restores them.
    Trashed(Vec<String>),
    /// Renamed (old path, new path).
    Renamed(PathBuf, PathBuf),
}

pub enum Report {
    Done {
        line: String,
        undo: Option<Undo>,
    },
    /// A folder, its bytes and file count.
    Size(PathBuf, u64, u64),
}

/// Running workers: their reports, the wake-up pipe, and the progress of each.
pub struct Jobs {
    tx: Sender<Report>,
    rx: Receiver<Report>,
    wake_r: File,
    wake_w: File,
    /// Copy and move jobs still running: what they do, and their progress.
    active: Vec<(String, Arc<Progress>)>,
    pub running: usize,
}

impl Jobs {
    pub fn new() -> io::Result<Self> {
        let (r, w) = frame::sys::pipe()?;
        frame::sys::set_nonblocking(r.as_fd())?;
        let (tx, rx) = channel();
        let (wake_r, wake_w) = (File::from(r), File::from(w));
        Ok(Jobs {
            tx,
            rx,
            wake_r,
            wake_w,
            active: Vec::new(),
            running: 0,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake_r.as_fd()
    }

    /// Run `op` on a worker thread.
    pub fn start(&mut self, op: Op) -> io::Result<()> {
        let mut wake = self.wake_w.try_clone()?;
        let tx = self.tx.clone();
        let prog = Arc::new(Progress::default());
        if let Some(label) = label(&op) {
            self.active.push((label, Arc::clone(&prog)));
        }
        self.running += 1;
        std::thread::spawn(move || {
            let _ = tx.send(run(op, &prog));
            drop(prog); // before waking: the window sees this job as finished
            let _ = wake.write_all(&[1]);
        });
        Ok(())
    }

    /// Reports that arrived (one per finished job).
    pub fn reports(&mut self) -> Vec<Report> {
        let mut sink = [0u8; 64];
        while matches!(self.wake_r.read(&mut sink), Ok(n) if n > 0) {}
        let out: Vec<Report> = self.rx.try_iter().collect();
        self.running = self.running.saturating_sub(out.len());
        // A finished worker dropped its handle: only ours is left.
        self.active.retain(|(_, p)| Arc::strong_count(p) > 1);
        out
    }

    /// The first running copy or move: what it does and how far (0-1).
    pub fn progress(&self) -> Option<(&str, f32)> {
        let (label, p) = self.active.first()?;
        let total = p.total.load(Relaxed).max(1);
        Some((label, (p.done.load(Relaxed) as f32 / total as f32).min(1.0)))
    }

    /// Stop every running copy and move (finished files stay, the half-done one goes).
    pub fn cancel(&self) {
        for (_, p) in &self.active {
            p.cancel.store(true, Relaxed);
        }
    }
}

/// The progress label of a job that shows progress.
fn label(op: &Op) -> Option<String> {
    let (verb, n) = match op {
        Op::Copy(s, ..) => ("Copying", s.len()),
        Op::Move(s, ..) => ("Moving", s.len()),
        _ => return None,
    };
    Some(if n == 1 {
        format!("{verb} 1 item")
    } else {
        format!("{verb} {n} items")
    })
}

/// What each item came to: its name (or source path) and the result.
type Results = Vec<(String, io::Result<Option<PathBuf>>)>;
/// Builds the undo from the (source, result) pairs of the items that were done.
type MakeUndo = fn(Vec<(PathBuf, PathBuf)>) -> Option<Undo>;

/// Do `op`.
fn run(op: Op, prog: &Progress) -> Report {
    let total = |src: &[PathBuf]| src.iter().map(|p| copy::measure(p).0).sum();
    let (verb, results, undo): (&str, Results, MakeUndo) = match op {
        Op::Copy(src, dir, c) => {
            prog.total.store(total(&src), Relaxed);
            (
                "Copied",
                each(&src, |p| copy::copy_into(p, &dir, c, prog)),
                made,
            )
        }
        Op::Move(src, dir, c) => {
            prog.total.store(total(&src), Relaxed);
            (
                "Moved",
                each(&src, |p| copy::move_into(p, &dir, c, prog)),
                |v| Some(Undo::Moved(v)),
            )
        }
        Op::Trash(src) => {
            let r = each(&src, |p| trash::put(p).map(|n| Some(PathBuf::from(n))));
            ("Moved to Trash:", r, trashed)
        }
        Op::Measure(p) => {
            let (bytes, files) = copy::measure(&p);
            return Report::Size(p, bytes, files);
        }
        other => return run_trash_and_archives(other),
    };
    summarized(verb, results, undo)
}

/// The jobs without progress: restore, erase, extract, compress, move back.
fn run_trash_and_archives(op: Op) -> Report {
    let erase = |n: &str| trash::erase(n).map(|()| None);
    let (verb, results, undo): (&str, Results, MakeUndo) = match op {
        Op::Restore(names) => (
            "Restored",
            each_name(&names, |n| trash::restore(n).map(Some)),
            made,
        ),
        Op::Erase(names) => ("Deleted", each_name(&names, erase), none),
        Op::EmptyTrash => ("Deleted", each_name(&trash::names(), erase), none),
        Op::Extract(a, dir) => (
            "Extracted",
            each(&[a], |p| archive::extract(p, &dir).map(Some)),
            made,
        ),
        Op::Compress(src, out) => {
            let r = archive::compress(&src, &out).map(|()| Some(out.clone()));
            ("Made", vec![(name_of(&out), r)], made)
        }
        Op::MoveBack(pairs) => {
            let back = |(from, to): &(PathBuf, PathBuf)| {
                (name_of(from), copy::move_back(to, from).map(|()| Some(from.clone())))
            };
            // Say where it went: the folder may not be on screen (owner: it "disappeared").
            let home = crate::view::home();
            let dir = pairs.first().and_then(|(from, _)| from.parent());
            let dir = dir.map_or(String::new(), |d| match d.strip_prefix(&home) {
                Ok(rest) => format!("~/{}", rest.display()),
                Err(_) => d.display().to_string(),
            });
            let mut r = summarized("Undone: moved back", pairs.iter().map(back).collect(), none);
            if let Report::Done { line, .. } = &mut r
                && !line.starts_with("Error")
            {
                line.push_str(&format!(" to {}", dir.trim_end_matches('/')));
            }
            return r;
        }
        Op::Copy(..) | Op::Move(..) | Op::Trash(_) | Op::Measure(_) => {
            unreachable!("run handles these")
        }
    };
    summarized(verb, results, undo)
}

fn trashed(v: Vec<(PathBuf, PathBuf)>) -> Option<Undo> {
    Some(Undo::Trashed(
        v.into_iter().map(|p| p.1.display().to_string()).collect(),
    ))
}

fn made(v: Vec<(PathBuf, PathBuf)>) -> Option<Undo> {
    Some(Undo::Made(v.into_iter().map(|p| p.1).collect()))
}

fn none(_: Vec<(PathBuf, PathBuf)>) -> Option<Undo> {
    None
}

/// The report: a summary line, and the undo built from (source, result) of the items
/// that were done.
fn summarized(
    verb: &str,
    results: Results,
    undo: impl Fn(Vec<(PathBuf, PathBuf)>) -> Option<Undo>,
) -> Report {
    let line = summary(verb, &results);
    let done: Vec<(PathBuf, PathBuf)> = results
        .into_iter()
        .filter_map(|(name, r)| r.ok().flatten().map(|to| (PathBuf::from(name), to)))
        .collect();
    let undo = if done.is_empty() { None } else { undo(done) };
    Report::Done { line, undo }
}

/// Each source path with what `f` did to it (the source is kept for undoing moves).
fn each(src: &[PathBuf], f: impl Fn(&Path) -> io::Result<Option<PathBuf>>) -> Results {
    src.iter()
        .map(|p| (p.display().to_string(), f(p)))
        .collect()
}

fn each_name(names: &[String], f: impl Fn(&str) -> io::Result<Option<PathBuf>>) -> Results {
    names.iter().map(|n| (n.clone(), f(n))).collect()
}

pub fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// "Copied “a.txt”", "Copied 3 items, skipped 1", or the first failure and how many
/// worked.
fn summary(verb: &str, results: &[(String, io::Result<Option<PathBuf>>)]) -> String {
    let short = |s: &str| name_of(Path::new(s));
    let ok = results
        .iter()
        .filter(|r| matches!(r.1, Ok(Some(_))))
        .count();
    let skipped = results.iter().filter(|r| matches!(r.1, Ok(None))).count();
    if let Some((name, Err(e))) = results.iter().find(|r| r.1.is_err()) {
        let rest = if ok > 0 {
            format!(" ({ok} done)")
        } else {
            String::new()
        };
        return format!("Error: \u{201c}{}\u{201d}: {e}{rest}", short(name));
    }
    let skip = if skipped > 0 && verb != "Deleted" {
        format!(", skipped {skipped}")
    } else {
        String::new()
    };
    match results {
        [(name, _)] if skipped == 0 => format!("{verb} \u{201c}{}\u{201d}", short(name)),
        _ => format!(
            "{verb} {} items{skip}",
            ok + if verb == "Deleted" { skipped } else { 0 }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn jobs_report_with_undo() {
        let d = std::env::temp_dir().join(format!("files-jobs-{}", std::process::id()));
        fs::create_dir_all(d.join("to")).unwrap();
        fs::write(d.join("a"), "1").unwrap();
        let mut jobs = Jobs::new().unwrap();
        jobs.start(Op::Move(
            vec![d.join("a")],
            d.join("to"),
            Conflict::KeepBoth,
        ))
        .unwrap();
        let mut got = Vec::new();
        for _ in 0..200 {
            got = jobs.reports();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        fs::remove_dir_all(&d).unwrap();
        let Some(Report::Done { line, undo }) = got.pop() else {
            panic!("no report")
        };
        assert_eq!(line, "Moved \u{201c}a\u{201d}");
        assert_eq!(undo, Some(Undo::Moved(vec![(d.join("a"), d.join("to/a"))])));
        assert_eq!((jobs.running, jobs.progress().is_none()), (0, true));
    }

    #[test]
    fn undoing_a_move_says_where_it_went() {
        let d = std::env::temp_dir().join(format!("files-moveback-{}", std::process::id()));
        fs::create_dir_all(d.join("into")).unwrap();
        fs::write(d.join("into/a.txt"), "x").unwrap();
        let op = Op::MoveBack(vec![(d.join("a.txt"), d.join("into/a.txt"))]);
        let line = match run_trash_and_archives(op) {
            Report::Done { line, .. } => line,
            _ => String::new(),
        };
        let back = d.join("a.txt").exists();
        fs::remove_dir_all(&d).unwrap();
        assert!(back);
        assert_eq!(line, format!("Undone: moved back \u{201c}a.txt\u{201d} to {}", d.display()));
    }
}
