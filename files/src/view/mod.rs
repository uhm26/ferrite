//! The window's content: Nemo's layout in frame's flat look. Places run down the
//! left (folding to icons in narrow windows); the toolbar has back, forward, up,
//! the path and the view switch; under it the folder's name, then its entries as a
//! list or a grid on one panel. Keys go to the [`Browser`] here; each part handles
//! its own clicks.

mod grid;
mod list;
mod location;
mod scroll;
mod sidebar;
mod toolbar;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use frame::ui::paint::mix;
use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, Rect, Ui};

use crate::browse::Browser;
use crate::dir::{self, Entry, Kind};
use grid::Grid;
use list::List;
use location::Location;
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
/// The toolbar band: one control with 8 above and below.
const BAR_H: f32 = CONTROL_H + 2.0 * S8;
/// The view switch: list or grid.
const LIST: usize = 0;

pub struct View {
    pub quit: bool,
    places: Places,
    scroll: Scroll,
    grid: Grid,
    list: List,
    loc: Location,
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
            list: List::default(),
            loc: Location::default(),
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

    /// Clipboard work the app must do after a frame: read it (Ctrl+V in the
    /// location field) and/or put text on it (Ctrl+C).
    pub fn clipboard(&mut self) -> (bool, Option<String>) {
        (
            std::mem::take(&mut self.loc.paste_wanted),
            self.loc.copy.take(),
        )
    }

    /// Clipboard text that arrived for the location field.
    pub fn paste(&mut self, text: String) {
        self.loc.paste(text);
    }

    /// The pointer is on the sidebar's resize edge (or dragging it).
    pub fn resize_cursor(&self) -> bool {
        self.places.resizing()
    }

    pub fn scroll_by(&mut self, discrete: i32, px: f64) {
        self.scroll.wheel(discrete, px);
    }

    pub fn draw(&mut self, ui: &mut Ui, b: &mut Browser) {
        let (gen0, open0) = (b.generation, self.loc.open);
        self.keys(ui, b);
        let (w, h) = ui.p.size();
        // The frame (places, toolbar, status bar) is one surface; the list sits in a
        // rounded pane set into it (owner: square inner corners looked "hard").
        let (side, window) = (ui.pal.sidebar, ui.pal.window);
        ui.p.clear(side);
        let nav_w = self.places.draw(ui, b);
        let cw = (w - nav_w).max(0.0);
        let pane = Rect::new(
            nav_w,
            BAR_H,
            (cw - S8).max(0.0),
            (h - BAR_H - ROW_H).max(0.0),
        );
        ui.p.rect(pane, R_PANEL, window);
        let bar = Rect::new(nav_w + S12, S8, cw - 2.0 * S12, CONTROL_H);
        if toolbar::draw(ui, b, bar, &mut self.mode, &mut self.loc) {
            self.scroll.follow();
        }
        status(ui, b, Rect::new(nav_w, h - ROW_H, cw, ROW_H));
        let panel = pane.inset(S4);
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
            self.list.draw(ui, b, panel, &mut self.scroll)
        } else {
            self.grid.draw(ui, b, panel, &mut self.scroll)
        };
        if let Some(i) = clicked {
            self.click(b, i);
        }
        // Changed after parts were drawn (a click opened a folder, the path bar came or
        // went): draw again, no event will ask for it.
        if b.generation != gen0 || self.loc.open != open0 {
            ui.redraw();
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

    /// Nemo keys: Ctrl+L (or typing / or ~) edits the path, Alt+←/→ history, Alt+↑ or
    /// Backspace up, Alt+Home home, Return
    /// opens, Ctrl+H hidden files. Home/End and PgUp/PgDn jump. Arrows (or hjkl) move: in the grid ←/→ step and
    /// ↑/↓ move a row; in the list ← goes up and → opens. Esc or q quits.
    fn keys(&mut self, ui: &mut Ui, b: &mut Browser) {
        if self.loc.open {
            return; // the location field types; its Ctrl keys are handled there
        }
        let grid = self.mode != LIST;
        let row = self.grid.cols.max(1) as isize;
        let page = self.page as isize;
        let mut keys = std::mem::take(&mut ui.s.input.keys).into_iter();
        while let Some(k) = keys.next() {
            match (k.mods.alt, k.sym, k.text.as_str()) {
                (true, KEY_LEFT, _) => b.travel(true),
                (true, KEY_RIGHT, _) => b.travel(false),
                (true, KEY_UP, _) | (_, KEY_BACKSPACE, _) => b.up(),
                (true, KEY_HOME, _) => b.go(home(), None),
                _ if k.mods.ctrl && k.sym == u32::from(b'h') => b.toggle_hidden(),
                _ if k.mods.ctrl && k.sym == u32::from(b'l') => self.loc.start(ui, b, None),
                (false, _, "/" | "~") if !k.mods.ctrl => self.loc.start(ui, b, Some(&k.text)),
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
            if self.loc.open {
                // Keys typed after `/` in the same frame belong to the field.
                ui.s.input.keys = keys.collect();
                return;
            }
            self.scroll.follow();
        }
    }
}

/// The status bar: item count (or the last error, in red) on the left, what is
/// selected on the right, like Nemo's.
fn status(ui: &mut Ui, b: &Browser, r: Rect) {
    let inner = Rect::new(r.x + S12, r.y, r.w - 2.0 * S12, r.h);
    let (note, color) = match &b.error {
        Some(e) => (e.clone(), ERROR),
        None => (count(b.entries.len(), b.hidden), ui.pal.sub),
    };
    let small = Ink::new(SMALL, REGULAR, color);
    let half = Rect {
        w: (inner.w / 2.0).round(),
        ..inner
    };
    ui.text(half, &note, small, Align::Left);
    if let Some(e) = b.entries.get(b.sel) {
        let what = if let (true, Some(n)) = (e.dir, e.items) {
            format!("\u{201c}{}\u{201d} selected ({})", e.name, dir::items(n))
        } else if e.dir {
            format!("\u{201c}{}\u{201d} selected", e.name)
        } else {
            format!(
                "\u{201c}{}\u{201d} selected ({})",
                e.name,
                dir::human(e.size)
            )
        };
        let right = Rect {
            x: half.x + half.w,
            ..half
        };
        let small = ui.small();
        ui.text(right, &what, small, Align::Right);
    }
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
        Kind::Folder => Icon::FolderFill,
        Kind::Image => Icon::Image,
        Kind::Audio => Icon::Music,
        Kind::Video => Icon::Video,
        Kind::Text => Icon::Documents,
        Kind::Code => Icon::Code,
        Kind::Archive | Kind::Other => Icon::File,
    }
}

/// Icon color: solid folders in a calm gray, file outlines in `sub`; both lift to
/// `text` as the row is selected (`sel` 0-1).
fn icon_color(ui: &Ui, e: &Entry, sel: f32) -> u32 {
    let rest = if e.dir {
        mix(ui.pal.dim, ui.pal.sub, 0.55)
    } else {
        ui.pal.sub
    };
    mix(rest, ui.pal.text, sel)
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
