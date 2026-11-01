//! The window's content: Nemo's layout in frame's flat look. Places run down the
//! left (folding to icons in narrow windows); the toolbar has back, forward, up,
//! the path and the view switch; under it the folder's name, then its entries as a
//! list or a grid on one panel. Keys go to the [`Browser`] here; each part handles
//! its own clicks.

mod grid;
mod list;
mod scroll;
mod sidebar;
mod toolbar;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, Rect, Ui};

use crate::browse::Browser;
use crate::dir::{self, Entry, Kind};
use grid::Grid;
use scroll::Scroll;
use sidebar::Places;

pub use sidebar::home;

/// xkbcommon keysyms (xkbcommon-keysyms.h).
const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_HOME: u32 = 0xff50;
const KEY_LEFT: u32 = 0xff51;
const KEY_UP: u32 = 0xff52;
const KEY_RIGHT: u32 = 0xff53;
const KEY_DOWN: u32 = 0xff54;
const KEY_PAGE_UP: u32 = 0xff55;
const KEY_PAGE_DOWN: u32 = 0xff56;
const KEY_END: u32 = 0xff57;

/// GTK's default double-click time.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Title line under the toolbar.
const TITLE_H: f32 = 32.0;
/// The view switch: list or grid.
const LIST: usize = 0;

pub struct View {
    pub quit: bool,
    places: Places,
    scroll: Scroll,
    grid: Grid,
    /// [`LIST`] or grid.
    mode: usize,
    last_click: Option<(usize, Instant)>,
    /// Entries one screen holds (PgUp/PgDn), from the last frame.
    page: usize,
    /// The folder shown last frame: any change (click, key, history) re-aims scrolling.
    shown: PathBuf,
}

impl View {
    pub fn new() -> Self {
        View {
            quit: false,
            places: Places::new(),
            scroll: Scroll::default(),
            grid: Grid::default(),
            mode: LIST,
            last_click: None,
            page: 1,
            shown: PathBuf::new(),
        }
    }

    /// Show the grid instead of the list.
    pub fn set_grid(&mut self, on: bool) {
        self.mode = usize::from(on);
    }

    pub fn scroll_by(&mut self, discrete: i32, px: f64) {
        self.scroll.wheel(discrete, px);
    }

    pub fn draw(&mut self, ui: &mut Ui, b: &mut Browser) {
        self.keys(ui, b);
        let (w, h) = ui.p.size();
        let window = ui.pal.window;
        ui.p.clear(window);
        let nav_w = self.places.draw(ui, b);
        let x = nav_w + S24;
        let cw = (w - x - S24).max(0.0);
        let bar = Rect::new(x, S16, cw, CONTROL_H);
        if toolbar::draw(ui, b, bar, &mut self.mode) {
            self.scroll.follow();
        }
        let ty = bar.y + bar.h + S12;
        title(ui, b, Rect::new(x, ty, cw, TITLE_H));
        let top = ty + TITLE_H + S12;
        let panel = Rect::new(x, top, cw, (h - top - S24).max(0.0));
        self.page = if self.mode == LIST {
            ((panel.h / ROW_H) as usize).saturating_sub(2)
        } else {
            self.grid.cols * ((panel.h / grid::TILE_H) as usize).saturating_sub(1)
        }
        .max(1);
        if self.shown != b.cwd {
            self.shown.clone_from(&b.cwd);
            self.scroll.follow();
        }
        let clicked = if self.mode == LIST {
            list::draw(ui, b, panel, &mut self.scroll)
        } else {
            self.grid.draw(ui, b, panel, &mut self.scroll)
        };
        if let Some(i) = clicked {
            self.click(b, i);
        }
    }

    /// Click selects; a second click on the same entry soon after opens it.
    fn click(&mut self, b: &mut Browser, i: usize) {
        let double = self
            .last_click
            .is_some_and(|(j, t)| j == i && t.elapsed() < DOUBLE_CLICK);
        b.sel = i;
        if double {
            self.last_click = None;
            b.open();
            self.scroll.follow();
        } else {
            self.last_click = Some((i, Instant::now()));
        }
    }

    /// Nemo keys: Alt+←/→ history, Alt+↑ or Backspace up, Alt+Home home, Return
    /// opens, Ctrl+H hidden files. Home/End and PgUp/PgDn jump. Arrows (or hjkl) move: in the grid ←/→ step and
    /// ↑/↓ move a row; in the list ← goes up and → opens. Esc or q quits.
    fn keys(&mut self, ui: &mut Ui, b: &mut Browser) {
        let grid = self.mode != LIST;
        let row = self.grid.cols.max(1) as isize;
        let page = self.page as isize;
        for k in std::mem::take(&mut ui.s.input.keys) {
            match (k.mods.alt, k.sym, k.text.as_str()) {
                (true, KEY_LEFT, _) => b.travel(true),
                (true, KEY_RIGHT, _) => b.travel(false),
                (true, KEY_UP, _) | (_, KEY_BACKSPACE, _) => b.up(),
                (true, KEY_HOME, _) => b.go(home(), None),
                _ if k.mods.ctrl && k.sym == u32::from(b'h') => b.toggle_hidden(),
                (_, KEY_ESCAPE, _) | (_, _, "q") => self.quit = true,
                (_, KEY_RETURN, _) => b.open(),
                (_, KEY_HOME, _) => b.step(-isize::MAX),
                (_, KEY_END, _) => b.step(isize::MAX),
                (_, KEY_PAGE_UP, _) => b.step(-page),
                (_, KEY_PAGE_DOWN, _) => b.step(page),
                (_, KEY_UP, _) | (_, _, "k") => b.step(if grid { -row } else { -1 }),
                (_, KEY_DOWN, _) | (_, _, "j") => b.step(if grid { row } else { 1 }),
                (_, KEY_LEFT, _) | (_, _, "h") if grid => b.step(-1),
                (_, KEY_RIGHT, _) | (_, _, "l") if grid => b.step(1),
                (_, KEY_LEFT, _) | (_, _, "h") => b.up(),
                (_, KEY_RIGHT, _) | (_, _, "l") => b.open(),
                _ => continue,
            }
            self.scroll.follow();
        }
    }
}

/// The folder's name, and how many entries it has (or the last error, in red).
fn title(ui: &mut Ui, b: &Browser, r: Rect) {
    let name = if b.cwd == home() {
        "Home".to_string()
    } else {
        b.cwd
            .file_name()
            .map_or("/".into(), |n| n.to_string_lossy().into_owned())
    };
    let ink = ui.title_ink();
    let room = (r.w * 0.6).round();
    let name_w = ui.s.text.width(&name, ink).min(room);
    ui.text(Rect { w: room, ..r }, &name, ink, Align::Left);
    let (note, color) = match &b.error {
        Some(e) => (e.clone(), ERROR),
        None => (count(b.entries.len(), b.hidden), ui.pal.sub),
    };
    let small = Ink::new(SMALL, REGULAR, color);
    let nx = r.x + name_w + S12;
    // Baseline-align the note with the title rather than centering it.
    let base = ui.s.text.center(r.y, r.h, ink);
    let note =
        ui.s.text
            .ellipsize(&note, r.x + r.w - nx, small)
            .into_owned();
    ui.label((nx, base), &note, small);
}

fn count(n: usize, hidden: bool) -> String {
    let what = match n {
        0 => "Empty".to_string(),
        1 => "1 item".to_string(),
        n => format!("{n} items"),
    };
    if hidden {
        format!("{what}, hidden files shown")
    } else {
        what
    }
}

/// The icon for an entry's kind.
fn icon(e: &Entry) -> Icon {
    match dir::kind(e).0 {
        Kind::Folder => Icon::Folder,
        Kind::Image => Icon::Image,
        Kind::Audio => Icon::Music,
        Kind::Video => Icon::Video,
        Kind::Text => Icon::Documents,
        Kind::Code => Icon::Code,
        Kind::Archive | Kind::Other => Icon::File,
    }
}

/// "This folder is empty", centered in `r` under a large folder.
fn empty(ui: &mut Ui, r: Rect) {
    let (sub, dim) = (ui.pal.sub, ui.pal.dim);
    let mid = r.y + (r.h / 2.0).round();
    ui.icon(
        Icon::Folder,
        Rect::new(r.x, mid - 48.0, r.w, 40.0),
        2.0 * ICON,
        dim,
    );
    let ink = Ink::new(BODY, REGULAR, sub);
    ui.text(
        Rect::new(r.x, mid, r.w, 20.0),
        "This folder is empty",
        ink,
        Align::Center,
    );
}
