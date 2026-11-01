//! The window's content: the path on top, the entries as rows in a card below.
//! Turns keys, clicks and the wheel into [`Browser`] moves.

use frame::font::{BOLD, REGULAR};
use frame::ui::theme::*;
use frame::ui::{Ink, Rect, Ui};

use crate::browse::Browser;
use crate::dir;

/// xkbcommon keysyms (xkbcommon-keysyms.h).
const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_LEFT: u32 = 0xff51;
const KEY_UP: u32 = 0xff52;
const KEY_RIGHT: u32 = 0xff53;
const KEY_DOWN: u32 = 0xff54;

/// Path line height: 16 above and below the heading.
const HEADER_H: f32 = 48.0;
const WHEEL_STEP: f32 = ROW_H;

#[derive(Default)]
pub struct View {
    pub quit: bool,
    /// Wanted scroll offset of the list, px.
    scroll: f32,
    /// Scroll so the selection is visible (after a key move, not after the wheel).
    follow: bool,
}

impl View {
    pub fn scroll_by(&mut self, discrete: i32, px: f64) {
        self.scroll += if discrete != 0 {
            discrete as f32 * WHEEL_STEP
        } else {
            px as f32
        };
        self.follow = false;
    }

    pub fn draw(&mut self, ui: &mut Ui, b: &mut Browser) {
        self.keys(ui, b);
        let (w, h) = ui.p.size();
        let bg = ui.pal.bg;
        ui.p.clear(bg);
        header(ui, b, w);
        let card = Rect::new(S12, HEADER_H, w - 2.0 * S12, h - HEADER_H - S12);
        ui.card(card);
        let list = card.inset(S8);
        self.rows(ui, b, list);
    }

    /// Shortcuts: arrows or hjkl move, Return opens, Backspace goes up, Esc quits.
    fn keys(&mut self, ui: &mut Ui, b: &mut Browser) {
        for k in std::mem::take(&mut ui.s.input.keys) {
            match (k.sym, k.text.as_str()) {
                (KEY_ESCAPE, _) | (_, "q") => self.quit = true,
                (KEY_UP, _) | (_, "k") => b.step(-1),
                (KEY_DOWN, _) | (_, "j") => b.step(1),
                (KEY_RETURN | KEY_RIGHT, _) | (_, "l") => b.open(),
                (KEY_BACKSPACE | KEY_LEFT, _) | (_, "h") => b.up(),
                _ => continue,
            }
            self.follow = true;
        }
    }

    /// The visible rows of `b` inside `list`. Click selects; click the selected one opens.
    fn rows(&mut self, ui: &mut Ui, b: &mut Browser, list: Rect) {
        if b.entries.is_empty() {
            let dim = Ink::new(BODY, REGULAR, ui.pal.dim);
            let y = ui.s.text.center(list.y, ROW_H, dim);
            ui.label((list.x + S12, y), "empty", dim);
            return;
        }
        let max = (b.entries.len() as f32 * ROW_H - list.h).max(0.0);
        if self.follow {
            let top = b.sel as f32 * ROW_H;
            // max then min, not clamp: a window shorter than a row would make lo > hi.
            self.scroll = self.scroll.max(top + ROW_H - list.h).min(top);
        }
        self.scroll = self.scroll.clamp(0.0, max);
        // One spring per folder: entering another one starts at its own offset, no glide.
        // shortcut: one spring per folder visited lives until exit, fine for a session.
        let key = format!("scroll/{}", b.cwd.display());
        let off = ui.spring(&key, self.scroll, SNAP).round();
        let first = (off / ROW_H) as usize;
        let last = (((off + list.h) / ROW_H).ceil() as usize).min(b.entries.len());
        ui.p.clip(Some(list));
        let mut clicked = None;
        for i in first..last {
            let e = &b.entries[i];
            let r = Rect::new(list.x, list.y + i as f32 * ROW_H - off, list.w, ROW_H);
            let name = if e.dir {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            };
            let hint = if e.dir {
                String::new()
            } else {
                dir::human(e.size)
            };
            // shortcut: a long name runs under its size hint; truncate with … when it bites.
            if ui.row(&format!("row/{i}"), r, (&name, &hint), i == b.sel) {
                clicked = Some(i);
            }
        }
        ui.p.clip(None);
        if let Some(i) = clicked {
            if i == b.sel {
                b.open();
                self.follow = true;
            } else {
                b.sel = i;
            }
        }
    }
}

/// The current path (home as `~`), and the last error on the right.
fn header(ui: &mut Ui, b: &Browser, w: f32) {
    let home = std::env::var_os("HOME").unwrap_or_default();
    let path = match b.cwd.strip_prefix(&home) {
        Ok(rest) if !home.is_empty() && rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) if !home.is_empty() => format!("~/{}", rest.display()),
        _ => b.cwd.display().to_string(),
    };
    let head = Ink::new(HEADING, BOLD, ui.pal.fg);
    let y = ui.s.text.center(0.0, HEADER_H, head);
    ui.label((S24, y), &path, head);
    if let Some(err) = &b.error {
        let ink = Ink::new(CAPTION, REGULAR, ERROR);
        let ew = ui.s.text.width(err, ink);
        let y = ui.s.text.center(0.0, HEADER_H, ink);
        ui.label((w - S24 - ew, y), err, ink);
    }
}
