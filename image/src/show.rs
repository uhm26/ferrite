//! What is on screen: the folder's images, the current one decoded (all frames of an
//! animation, an SVG rendered to fit), its [`View`], and painting all that into a
//! window buffer; plus what can be done to the current image (turn, trash, copy).
//! No window here, so `--png` and tests use it too.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use frame::ui::Text;
use frame::ui::paint::Painter;
use frame::ui::text::{Ink, MEDIUM};
use frame::ui::theme::{BODY, SUB, WINDOW};
use frame::window::Canvas;
use pix::Mip;

use crate::load::{Loaded, Near};
use crate::view::View;
use crate::{folder, trash};

pub struct Show {
    text: Text,
    list: Vec<PathBuf>,
    at: usize,
    pub img: Loaded,
    /// The neighbors, decoding or decoded.
    near: Near,
    frame: usize,
    /// When the next animation frame is due; None for a still.
    due: Option<Instant>,
    pub view: View,
    /// A word for the title ("copied", why trashing failed); cleared on step.
    pub note: String,
}

impl Show {
    pub fn open(path: &Path) -> Result<Self, frame::font::Error> {
        let (list, at) = folder::siblings(path);
        let mut s = Show {
            text: Text::new(frame::ui::theme::FAMILY)?,
            list,
            at,
            img: Err(String::new()),
            near: Near::default(),
            frame: 0,
            due: None,
            view: View::new(),
            note: String::new(),
        };
        s.load(None);
        Ok(s)
    }

    /// Show image `at`; `leaving` (the image shown until now) stays decoded nearby.
    fn load(&mut self, leaving: Option<PathBuf>) {
        let old = std::mem::replace(&mut self.img, Err(String::new()));
        if let Some(p) = leaving {
            self.near.keep(p, old);
        }
        self.img = match self.list.get(self.at) {
            Some(p) => self.near.take(p),
            None => Err("no images here".into()),
        };
        let at = self.at;
        let want: Vec<&Path> = [at.wrapping_sub(1), at + 1]
            .iter()
            .filter_map(|&i| self.list.get(i).map(PathBuf::as_path))
            .collect();
        self.near.only(&want);
        self.reset();
    }

    /// From the first frame, at fit.
    fn reset(&mut self) {
        self.view = View::new();
        self.note.clear();
        self.frame = 0;
        self.due = match &self.img {
            Ok(f) if f.len() > 1 => Some(Instant::now() + Duration::from_millis(f[0].1.into())),
            _ => None,
        };
    }

    /// The frame on screen.
    fn current(&self) -> Result<&Mip, &str> {
        match &self.img {
            Ok(f) => Ok(&f[self.frame].0),
            Err(e) => Err(e),
        }
    }

    /// When an animation wants its next frame (for the loop's poll timeout).
    pub fn due(&self) -> Option<Instant> {
        self.due
    }

    /// Show the next animation frame if it is due at `now`. True if it changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let (Some(due), Ok(f)) = (self.due, &self.img) else {
            return false;
        };
        if now < due {
            return false;
        }
        self.frame = (self.frame + 1) % f.len();
        let next = due + Duration::from_millis(f[self.frame].1.into());
        // After a stall (hidden window, slow frame) restart the clock, don't race to catch up.
        self.due = Some(if next < now { now } else { next });
        true
    }

    /// Move `by` images, stopping at the ends (`isize::MAX` = to the end).
    /// True if another image is now shown.
    pub fn step(&mut self, by: isize) -> bool {
        let last = self.list.len().saturating_sub(1) as isize;
        let to = (self.at as isize).saturating_add(by).clamp(0, last) as usize;
        if to == self.at {
            return false;
        }
        let leaving = self.list.get(self.at).cloned();
        self.at = to;
        self.load(leaving);
        true
    }

    /// Turn every frame by EXIF orientation `o` ([`pix::CW`], [`pix::CCW`]), back to
    /// fit. Only in memory: the file is never touched. True if there was an image.
    pub fn rotate(&mut self, o: u8) -> bool {
        let Ok(f) = &mut self.img else {
            return false;
        };
        for (m, _) in f.iter_mut() {
            *m = Mip::new(pix::orient(m.full(), o));
        }
        // Spin from wherever the picture shows now (a second r mid-turn adds on).
        let quarter = if o == pix::CW { 90.0 } else { -90.0 };
        self.view = View::new().turned(quarter - self.view.turn_left());
        true
    }

    /// Move the current file to the trash and show the next one (or the one before,
    /// at the end). A failure only shows in the title.
    pub fn trash(&mut self) {
        let Some(p) = self.list.get(self.at).cloned() else {
            return;
        };
        if let Err(e) = trash::put(&p) {
            self.note = format!("can't trash: {e}");
            return;
        }
        self.list.remove(self.at);
        self.at = self.at.min(self.list.len().saturating_sub(1));
        self.load(None);
    }

    /// The frame on screen as PNG bytes (transparency already on the background).
    pub fn png(&self) -> Result<Vec<u8>, String> {
        let m = self.current()?;
        pix::png(m.full()).map_err(|e| e.to_string())
    }

    /// The current image's size, if it decoded.
    pub fn size(&self) -> Option<(usize, usize)> {
        let img = self.current().ok()?.full();
        Some((img.w, img.h))
    }

    /// The window title: file name, place in the folder, size, zoom and any note.
    pub fn title(&self) -> String {
        let name = self.list.get(self.at).and_then(|p| p.file_name());
        let name = name.map_or("image".into(), |n| n.to_string_lossy());
        let n = self.list.len();
        let at = (self.at + 1).min(n);
        let mut t = match self.size() {
            Some((w, h)) => format!(
                "{name} — {at}/{n} — {w}×{h} — {:.0} %",
                self.view.zoom * 100.0
            ),
            None => format!("{name} — {at}/{n}"),
        };
        if !self.note.is_empty() {
            t += &format!(" — {}", self.note);
        }
        t
    }

    pub fn paint(&mut self, c: Canvas) {
        let (w, h) = (c.w, c.h);
        let frame = self.frame;
        match &self.img {
            Ok(f) => self.view.paint(&f[frame].0, (c.px, w, h)),
            Err(e) => {
                let mut p = Painter::new(c);
                p.clear(WINDOW);
                let ink = Ink::new(BODY, MEDIUM, SUB);
                let tw = self.text.width(e, ink);
                let y = self.text.center(0.0, h as f32, ink);
                self.text
                    .draw(&mut p, (((w as f32 - tw) / 2.0).round(), y), e, ink);
            }
        }
    }
}
