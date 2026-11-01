//! What is on screen: the folder's images, the current one decoded, its [`View`],
//! and painting all that into a window buffer. No window here, so `--png` and
//! tests use it too.

use std::path::{Path, PathBuf};

use frame::ui::Text;
use frame::ui::paint::Painter;
use frame::ui::text::{Ink, MEDIUM};
use frame::ui::theme::{BODY, SUB, WINDOW};
use frame::window::Canvas;

use crate::folder;
use crate::image::{self, Rgb};
use crate::view::View;

pub struct Show {
    text: Text,
    list: Vec<PathBuf>,
    at: usize,
    /// The current image, or why it can't be shown.
    pub img: Result<Rgb, String>,
    pub view: View,
    scratch: Vec<u32>,
}

impl Show {
    pub fn open(path: &Path) -> Result<Self, frame::font::Error> {
        let (list, at) = folder::siblings(path);
        let mut s = Show {
            text: Text::new(frame::ui::theme::FAMILY)?,
            list,
            at,
            img: Err(String::new()),
            view: View::new(),
            scratch: Vec::new(),
        };
        s.load();
        Ok(s)
    }

    fn load(&mut self) {
        self.view = View::new();
        self.img = match self.list.get(self.at) {
            Some(p) => image::load(p).map_err(|e| e.to_string()),
            None => Err("no PNG or JPEG images here".into()),
        };
    }

    /// Move `by` images, stopping at the ends (`isize::MAX` = to the end).
    /// True if another image is now shown.
    pub fn step(&mut self, by: isize) -> bool {
        let last = self.list.len().saturating_sub(1) as isize;
        let to = (self.at as isize).saturating_add(by).clamp(0, last) as usize;
        if to == self.at {
            return false;
        }
        self.at = to;
        self.load();
        true
    }

    /// The window title: file name, place in the folder, size and zoom.
    pub fn title(&self) -> String {
        let name = self.list.get(self.at).and_then(|p| p.file_name());
        let name = name.map_or("image".into(), |n| n.to_string_lossy());
        let n = self.list.len();
        let at = (self.at + 1).min(n);
        match &self.img {
            Ok(i) => format!(
                "{name} — {at}/{n} — {}×{} — {:.0} %",
                i.w,
                i.h,
                self.view.zoom * 100.0
            ),
            Err(_) => format!("{name} — {at}/{n}"),
        }
    }

    pub fn paint(&mut self, c: Canvas) {
        let (w, h) = (c.w, c.h);
        match &self.img {
            Ok(img) => self.view.paint(img, (c.px, w, h), &mut self.scratch),
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
