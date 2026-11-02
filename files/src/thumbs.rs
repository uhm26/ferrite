//! Thumbnails: pictures shrunk to fit a box, made on worker threads so the window
//! never waits. The grid asks while drawing ([`Thumbs::get`]); a finished one wakes
//! the loop through a pipe ([`Thumbs::fd`], then [`Thumbs::drain`]). Kept in memory
//! only, keyed by path, change time and box size (an edited picture is made again).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

/// A picture `w * h`, opaque ARGB.
pub struct Thumb {
    pub px: Vec<u32>,
    pub w: usize,
    pub h: usize,
}

/// Path, its change time, and the box side the thumbnail fits.
type Key = (PathBuf, i64, usize);

/// Threads making thumbnails (each resample also spreads over all cores).
const WORKERS: usize = 3;
/// Bigger files aren't read for a thumbnail.
const MAX_FILE: u64 = 64 << 20;
/// Jobs waiting at most: older ones are dropped (scrolled past) and asked again
/// when seen again.
const MAX_QUEUE: usize = 128;
/// Thumbnails kept at most; past this the made ones are all dropped.
// shortcut: drops everything at the cap instead of the least recently used, fine
// unless folders with thousands of pictures thrash; then keep an LRU order.
const MAX_KEPT: usize = 2000;
/// What transparent pictures are laid on: the pane's color.
const BG: [u8; 3] = [0x11; 3];

enum Slot {
    Waiting,
    Done(Option<Arc<Thumb>>),
}

type Queue = Arc<(Mutex<Vec<Key>>, Condvar)>;

pub struct Thumbs {
    slots: HashMap<Key, Slot>,
    queue: Queue,
    rx: Receiver<(Key, Option<Thumb>)>,
    wake_r: File,
}

impl Thumbs {
    pub fn new() -> io::Result<Self> {
        let (r, w) = frame::sys::pipe()?;
        frame::sys::set_nonblocking(r.as_fd())?;
        let (tx, rx) = channel();
        let queue: Queue = Arc::default();
        let wake = File::from(w);
        for _ in 0..WORKERS {
            let (q, tx, wake) = (Arc::clone(&queue), tx.clone(), wake.try_clone()?);
            std::thread::spawn(move || work(&q, &tx, wake));
        }
        Ok(Thumbs {
            slots: HashMap::new(),
            queue,
            rx,
            wake_r: File::from(r),
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake_r.as_fd()
    }

    /// The thumbnail of `path` (changed at `mtime`, unix seconds) fitting a `side` px box, if
    /// made; else it is asked for (None, also for files that aren't pictures).
    pub fn get(&mut self, path: &Path, mtime: i64, side: usize) -> Option<Arc<Thumb>> {
        let key = (path.to_path_buf(), mtime, side);
        match self.slots.get(&key) {
            Some(Slot::Done(t)) => return t.clone(),
            Some(Slot::Waiting) => return None,
            None => {}
        }
        if self.slots.len() >= MAX_KEPT {
            self.slots.retain(|_, s| matches!(s, Slot::Waiting));
        }
        let (lock, cv) = &*self.queue;
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        q.push(key.clone());
        if q.len() > MAX_QUEUE {
            let extra = q.len() - MAX_QUEUE;
            for old in q.drain(..extra) {
                self.slots.remove(&old);
            }
        }
        self.slots.insert(key, Slot::Waiting);
        cv.notify_one();
        None
    }

    /// Take in finished thumbnails (after the pipe woke the loop). True if any.
    pub fn drain(&mut self) -> bool {
        let mut sink = [0u8; 64];
        while matches!(self.wake_r.read(&mut sink), Ok(n) if n > 0) {}
        let mut any = false;
        for (key, t) in self.rx.try_iter() {
            // A slot dropped meanwhile (queue cap, cache cap) stays dropped.
            if let Some(s) = self.slots.get_mut(&key) {
                *s = Slot::Done(t.map(Arc::new));
                any = true;
            }
        }
        any
    }
}

/// A worker: take the newest job (what is on screen now), make it, report, wake.
fn work(q: &Queue, tx: &Sender<(Key, Option<Thumb>)>, mut wake: File) {
    let (lock, cv) = &**q;
    loop {
        let key = {
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                match q.pop() {
                    Some(k) => break k,
                    None => q = cv.wait(q).unwrap_or_else(|e| e.into_inner()),
                }
            }
        };
        let t = make(&key.0, key.2);
        if tx.send((key, t)).is_err() {
            return; // the window is gone
        }
        let _ = wake.write_all(&[1]);
    }
}

/// Decode `path` and shrink it to fit a `side` px square (never enlarged).
fn make(path: &Path, side: usize) -> Option<Thumb> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE {
        return None;
    }
    let img = pix::load(path, BG).ok()?;
    Some(fit(img, side))
}

fn fit(img: pix::Rgb, side: usize) -> Thumb {
    let s = (side as f64 / img.w.max(img.h) as f64).min(1.0);
    let (w, h) = (
        ((img.w as f64 * s).round() as usize).max(1),
        ((img.h as f64 * s).round() as usize).max(1),
    );
    let mut px = vec![0u32; w * h];
    let mip = pix::Mip::new(img);
    let (level, k) = mip.pick(s);
    pix::resample(level, s * k, (0.0, 0.0), (w, h), (&mut px, w));
    Thumb { px, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_shape_and_never_enlarges() {
        let rgb = |w, h| pix::Rgb {
            w,
            h,
            px: vec![200; w * h * 3],
        };
        let wide = fit(rgb(400, 100), 64);
        let tiny = fit(rgb(10, 20), 64);
        assert_eq!((wide.w, wide.h), (64, 16));
        assert_eq!((tiny.w, tiny.h), (10, 20));
        assert_eq!(wide.px[0] & 0xff, 200);
    }

    #[test]
    fn a_picture_arrives_and_a_non_picture_is_none() {
        let dir = std::env::temp_dir().join(format!("files-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        let img = pix::Rgb {
            w: 8,
            h: 4,
            px: vec![255; 8 * 4 * 3],
        };
        std::fs::write(&png, pix::png(&img).unwrap()).unwrap();
        let txt = dir.join("b.txt");
        std::fs::write(&txt, "no").unwrap();
        let mut t = Thumbs::new().unwrap();
        let now = 0;
        assert!(t.get(&png, now, 64).is_none(), "asked, not made yet");
        t.get(&txt, now, 64);
        let start = std::time::Instant::now();
        while t.slots.values().any(|s| matches!(s, Slot::Waiting)) {
            assert!(start.elapsed().as_secs() < 5, "workers stuck");
            std::thread::sleep(std::time::Duration::from_millis(5));
            t.drain();
        }
        let made = t.get(&png, now, 64).map(|p| (p.w, p.h));
        let none = t.get(&txt, now, 64).is_none();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(made, Some((8, 4)));
        assert!(none);
    }
}
