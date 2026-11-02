//! Decoding, mostly off the main thread: the images next to the shown one decode in
//! the background, so stepping to them is instant, and the one just left stays
//! decoded for stepping back.
//! shortcut: holds up to 3 decoded images (a long animation can be ~1 GiB each);
//! cap by bytes if memory ever bites.

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use frame::ui::theme::WINDOW;
use pix::Mip;

use crate::view::snug;

/// What shows through transparent pixels: the window color, as RGB.
pub const BG: [u8; 3] = [(WINDOW >> 16) as u8, (WINDOW >> 8) as u8, WINDOW as u8];

/// One image's frames (one for a still) with how long each shows, ms; or why it
/// can't be shown.
pub type Loaded = Result<Vec<(Mip, u32)>, String>;

/// Decode `p` now.
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

enum Slot {
    Ready(Loaded),
    Coming(JoinHandle<Loaded>),
}

/// Decoded (or decoding) images by path.
#[derive(Default)]
pub struct Near {
    slots: Vec<(PathBuf, Slot)>,
}

impl Near {
    /// `p` decoded: from here (waiting for its thread if still busy), else now.
    pub fn take(&mut self, p: &Path) -> Loaded {
        let Some(i) = self.slots.iter().position(|(q, _)| q == p) else {
            return decode(p);
        };
        match self.slots.swap_remove(i).1 {
            Slot::Ready(l) => l,
            Slot::Coming(h) => h.join().unwrap_or_else(|_| Err("decoder crashed".into())),
        }
    }

    /// Hold on to `l`, the decoded `p`.
    pub fn keep(&mut self, p: PathBuf, l: Loaded) {
        self.slots.push((p, Slot::Ready(l)));
    }

    /// Hold only the images in `want`; start decoding the ones not here yet.
    /// A dropped busy thread finishes on its own and its result is thrown away.
    pub fn only(&mut self, want: &[&Path]) {
        self.slots.retain(|(p, _)| want.contains(&p.as_path()));
        for &p in want {
            if self.slots.iter().any(|(q, _)| q == p) {
                continue;
            }
            let owned = p.to_path_buf();
            let job = std::thread::Builder::new().name("decode".into());
            // No thread (out of resources): it decodes when stepped to instead.
            if let Ok(h) = job.spawn(move || decode(&owned)) {
                self.slots.push((p.to_path_buf(), Slot::Coming(h)));
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
        near.keep(dir.join("gone.png"), Err("old".into()));
        near.only(&[&a, &b]);
        let paths: Vec<_> = near.slots.iter().map(|(p, _)| p.clone()).collect();
        let got_a = near.take(&a).map(|f| f[0].0.full().w);
        let got_b = near.take(&b);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(paths, [a, b]);
        assert_eq!(got_a, Ok(2));
        assert!(got_b.is_err());
        assert!(near.slots.is_empty());
    }
}
