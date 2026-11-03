//! Decoding, off the main thread. Every image decodes in its own thread and hands its
//! frames over a short channel ([`Stream`]): the first frame shows as soon as it is
//! ready, and the rest of an animation keeps arriving while it plays. A neighbor's thread
//! stalls once the channel is full, so the images next to the shown one cost at most
//! [`AHEAD`] frames each until stepped to. The one just left stays for stepping back,
//! unless it is big ([`KEEP_MAX`]); then it decodes again on the way back.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use frame::ui::theme::WINDOW;
use pix::Mip;

use crate::view::snug;

/// What shows through transparent pixels: the window color, as RGB.
pub const BG: [u8; 3] = [(WINDOW >> 16) as u8, (WINDOW >> 8) as u8, WINDOW as u8];

/// Frames a thread may decode ahead of what was taken from it.
const AHEAD: usize = 4;
/// The image just left is kept only up to this many bytes (mips included).
const KEEP_MAX: usize = 256 << 20;

/// One image's frames (one for a still) with how long each shows, ms; or why it
/// can't be shown.
pub type Loaded = Result<Vec<(Mip, u32)>, String>;
type Msg = Result<(Mip, u32), String>;

/// Decode `p` now, all of it.
pub fn decode(p: &Path) -> Loaded {
    pix::open(p, BG).and_then(frames).map_err(|e| e.to_string())
}

/// Mips for every frame. An SVG is rendered at the size its window will get, so it
/// shows 1:1. shortcut: zooming into an SVG enlarges pixels; re-render on zoom if asked.
fn frames(img: pix::Image) -> Result<Vec<(Mip, u32)>, pix::Error> {
    match img {
        pix::Image::Frames(f) => Ok(f.into_iter().map(|f| (Mip::new(f.img), f.ms)).collect()),
        pix::Image::Svg(s) => {
            let (w, h) = s.size();
            let size = snug((w.round().max(1.0) as usize, h.round().max(1.0) as usize));
            Ok(vec![(Mip::new(s.render(size, BG)?), 0)])
        }
    }
}

/// The decode thread: every frame into `tx` until the file ends or nobody listens.
fn send_all(p: &Path, tx: &SyncSender<Msg>) {
    let sent = pix::open_each(p, BG, &mut |f| tx.send(Ok((Mip::new(f.img), f.ms))).is_ok());
    let last = match sent {
        Ok(pix::Image::Frames(_)) => return,
        Ok(svg) => frames(svg).map(|mut f| f.swap_remove(0)),
        Err(e) => Err(e),
    };
    let _ = tx.send(last.map_err(|e| e.to_string()));
}

/// One image: the frames here so far, and the thread still sending more (if any).
pub struct Stream {
    pub got: Loaded,
    rx: Option<Receiver<Msg>>,
}

impl Stream {
    /// Start decoding `p` on a thread. No thread (out of resources): decode it here.
    pub fn start(p: &Path) -> Self {
        let (tx, rx) = sync_channel(AHEAD);
        let owned = p.to_path_buf();
        let job = std::thread::Builder::new().name("decode".into());
        match job.spawn(move || send_all(&owned, &tx)) {
            Ok(_) => Stream {
                got: Ok(Vec::new()),
                rx: Some(rx),
            },
            Err(_) => Stream::ready(decode(p)),
        }
    }

    /// Already decoded (or failed).
    pub fn ready(got: Loaded) -> Self {
        Stream { got, rx: None }
    }

    /// No more frames will come.
    pub fn done(&self) -> bool {
        self.rx.is_none()
    }

    /// Block until the first frame is here, or it is clear there will be none.
    pub fn first(&mut self) {
        while matches!(&self.got, Ok(f) if f.is_empty()) {
            match self.rx.as_ref().map(Receiver::recv) {
                Some(Ok(m)) => self.push(m),
                _ => {
                    self.rx = None;
                    self.got = Err("decoder crashed".into());
                }
            }
        }
    }

    /// Take whatever frames arrived, without waiting.
    pub fn pull(&mut self) {
        while let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(m) => self.push(m),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => self.rx = None,
            }
        }
    }

    fn push(&mut self, m: Msg) {
        match (&mut self.got, m) {
            (Ok(f), Ok(frame)) => f.push(frame),
            (Ok(f), Err(e)) if f.is_empty() => self.got = Err(e),
            _ => {} // an error after frames: keep what decoded
        }
    }

    /// Memory the decoded frames take.
    fn bytes(&self) -> usize {
        let Ok(f) = &self.got else { return 0 };
        f.first()
            .map_or(0, |(m, _)| m.full().px.len() * 4 / 3 * f.len())
    }
}

/// Decoded (or decoding) images by path.
#[derive(Default)]
pub struct Near {
    slots: Vec<(PathBuf, Stream)>,
}

impl Near {
    /// `p` with at least its first frame: from here if it was started, else started now.
    pub fn take(&mut self, p: &Path) -> Stream {
        let mut s = match self.slots.iter().position(|(q, _)| q == p) {
            Some(i) => self.slots.swap_remove(i).1,
            None => Stream::start(p),
        };
        s.first();
        s
    }

    /// Hold on to `s`, the image `p`, unless it is too big to keep around.
    pub fn keep(&mut self, p: PathBuf, s: Stream) {
        if s.bytes() <= KEEP_MAX {
            self.slots.push((p, s));
        }
    }

    /// Hold only the images in `want`; start decoding the ones not here yet.
    /// A dropped thread stops at its next frame.
    pub fn only(&mut self, want: &[&Path]) {
        self.slots.retain(|(p, _)| want.contains(&p.as_path()));
        for &p in want {
            if !self.slots.iter().any(|(q, _)| q == p) {
                self.slots.push((p.to_path_buf(), Stream::start(p)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_wanted_and_decodes_ahead() {
        let dir = std::env::temp_dir().join(format!("image-near-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.png"), dir.join("b.txt"));
        let img = pix::Rgb {
            w: 2,
            h: 1,
            px: vec![9; 6],
        };
        std::fs::write(&a, pix::png(&img).unwrap()).unwrap();
        std::fs::write(&b, "not an image").unwrap();
        let mut near = Near::default();
        near.keep(dir.join("gone.png"), Stream::ready(Err("old".into())));
        near.only(&[&a, &b]);
        let paths: Vec<_> = near.slots.iter().map(|(p, _)| p.clone()).collect();
        let got_a = near.take(&a).got.map(|f| f[0].0.full().w);
        let got_b = near.take(&b).got;
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(paths, [a, b]);
        assert_eq!(got_a, Ok(2));
        assert!(got_b.is_err());
        assert!(near.slots.is_empty());
    }

    /// A GIF of `n` 1×1 frames, 50 ms each.
    fn gif(n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = gif::Encoder::new(&mut out, 1, 1, &[0, 0, 0, 255, 255, 255]).unwrap();
        for i in 0..n {
            let mut f = gif::Frame::from_indexed_pixels(1, 1, vec![(i % 2) as u8], None);
            f.delay = 5;
            e.write_frame(&f).unwrap();
        }
        drop(e);
        out
    }

    #[test]
    fn streams_frames_and_a_neighbor_waits_after_a_few() {
        let dir = std::env::temp_dir().join(format!("image-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.gif");
        std::fs::write(&p, gif(20)).unwrap();
        let mut s = Stream::start(&p);
        s.first();
        // Nobody pulls: the thread stops once the channel is full.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!s.done());
        s.pull();
        let early = s.got.as_ref().map_or(0, Vec::len);
        while !s.done() {
            s.pull();
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            (2..=AHEAD + 2).contains(&early),
            "{early} frames before pulling"
        );
        assert_eq!(s.got.map(|f| f.len()), Ok(20));
    }
}
