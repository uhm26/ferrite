//! The window's content: Nemo's layout in frame's flat look. Places run down the
//! left (folding to icons in narrow windows); the toolbar has back, forward, up,
//! the path (or a filter) and the view switch; the entries sit in a rounded pane as
//! a list or a grid; a status bar closes the frame. Right-click opens a menu of
//! actions (actions.rs); keys are in keys.rs.

mod actions;
mod grid;
mod keys;
mod list;
mod location;
mod scroll;
mod sidebar;
mod toolbar;

use std::ffi::OsString;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use frame::ui::paint::mix;
use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, Rect, Ui};

use crate::browse::Browser;
use crate::dir::{self, Entry, Kind};
use crate::ops::Jobs;
use actions::{Clip, Menu, Rename};
use grid::Grid;
use list::List;
use location::Location;
use scroll::Scroll;
use sidebar::Places;

pub use sidebar::home;

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
    jobs: Jobs,
    clip: Option<Clip>,
    menu: Option<Menu>,
    rename: Option<Rename>,
    /// A right-click waiting for the next frame (which knows what is under it).
    right_click: Option<(f32, f32)>,
    /// Typed to find a name (type-to-find), and whether nothing matched.
    find: String,
    find_miss: bool,
    /// The last thing that happened ("Copied “a”", "Error: …"), until the next one.
    note: Option<String>,
    /// Delete Forever asked about this trashed item; the same again does it.
    confirm: Option<OsString>,
    /// Text for the system clipboard (a copied path).
    copy_text: Option<String>,
}

impl View {
    pub fn new() -> io::Result<Self> {
        Ok(View {
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
            jobs: Jobs::new()?,
            clip: None,
            menu: None,
            rename: None,
            right_click: None,
            find: String::new(),
            find_miss: false,
            note: None,
            confirm: None,
            copy_text: None,
        })
    }

    /// Show the grid instead of the list.
    pub fn set_grid(&mut self, on: bool) {
        self.mode = usize::from(on);
    }

    /// Clipboard work the app must do after a frame: read it (Ctrl+V in the
    /// location field) and/or put text on it (a copied path, or field text).
    pub fn clipboard(&mut self) -> (bool, Option<String>) {
        let copy = self.loc.copy.take().or_else(|| self.copy_text.take());
        (std::mem::take(&mut self.loc.paste_wanted), copy)
    }

    /// Clipboard text that arrived for the location field.
    pub fn paste(&mut self, text: String) {
        self.loc.paste(text);
    }

    /// The pointer is on the sidebar's resize edge (or dragging it).
    pub fn resize_cursor(&self) -> bool {
        self.places.resizing()
    }

    /// The pipe finished file jobs report through (the app polls it).
    pub fn jobs_fd(&self) -> BorrowedFd<'_> {
        self.jobs.fd()
    }

    /// The right button went down at `pos`: a menu opens on the next frame.
    pub fn right_click(&mut self, pos: (f32, f32)) {
        self.right_click = Some(pos);
    }

    pub fn scroll_by(&mut self, discrete: i32, px: f64) {
        if self.menu.is_none() {
            self.scroll.wheel(discrete, px);
        }
    }

    pub fn draw(&mut self, ui: &mut Ui, b: &mut Browser) {
        let (gen0, open0) = (b.generation, self.loc.open);
        self.keys(ui, b);
        // While the menu is open the content below must not see the pointer.
        let pointer = self.menu.is_some().then(|| hide_pointer(ui));
        self.frame(ui, b);
        if let Some(saved) = pointer {
            restore_pointer(ui, saved);
        }
        if let Some(at) = self.right_click.take() {
            let n = b.entries.len();
            let on = if self.mode == LIST {
                self.list.at(at, n)
            } else {
                self.grid.at(at, n)
            };
            self.open_menu(ui, at, on, b);
        }
        let name = self.item_rects(b.sel).map(|r| r.1);
        if self.rename.is_some() {
            self.rename(ui, b, name);
        }
        self.menu(ui, b);
        // Changed after parts were drawn (a click opened a folder, a bar came or
        // went): draw again, no event will ask for it.
        if b.generation != gen0 || self.loc.open != open0 {
            ui.redraw();
        }
    }

    /// Everything but the menu and the rename field.
    fn frame(&mut self, ui: &mut Ui, b: &mut Browser) {
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
        self.status(ui, b, Rect::new(nav_w, h - ROW_H, cw, ROW_H));
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
            self.find.clear();
            (self.confirm, self.note) = (None, None);
        }
        let clicked = if self.mode == LIST {
            self.list.draw(ui, b, panel, &mut self.scroll)
        } else {
            self.grid.draw(ui, b, panel, &mut self.scroll)
        };
        if let Some(by) = self.list.sort_click.take() {
            b.sort_by(by);
        }
        if let Some(i) = clicked {
            self.click(b, i);
        }
    }

    /// Entry `i`'s row (or tile) and name cell, from last frame's layout.
    fn item_rects(&self, i: usize) -> Option<(Rect, Rect)> {
        if self.mode == LIST {
            self.list.rects(i)
        } else {
            self.grid.rects(i)
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

    /// The status bar, like Nemo's: on the left what just happened, the find text,
    /// the filter or the item count (errors in red); on the right the selection.
    fn status(&self, ui: &mut Ui, b: &Browser, r: Rect) {
        let inner = Rect::new(r.x + S12, r.y, r.w - 2.0 * S12, r.h);
        let (left, red) = self.status_text(b);
        let small = Ink::new(SMALL, REGULAR, if red { ERROR } else { ui.pal.sub });
        let half = Rect {
            w: (inner.w / 2.0).round(),
            ..inner
        };
        ui.text(half, &left, small, Align::Left);
        if let Some(e) = b.entries.get(b.sel) {
            let right = Rect {
                x: half.x + half.w,
                ..half
            };
            let small = ui.small();
            ui.text(right, &selected(e), small, Align::Right);
        }
    }

    fn status_text(&self, b: &Browser) -> (String, bool) {
        if !self.find.is_empty() {
            return (format!("Find: {}", self.find), self.find_miss);
        }
        if let Some(e) = &b.error {
            return (e.clone(), true);
        }
        if self.jobs.running > 0 {
            return ("Working…".into(), false);
        }
        if let Some(n) = &self.note {
            return (n.clone(), n.starts_with("Error"));
        }
        let count = count(b.entries.len(), b.hidden);
        if b.filter.is_empty() {
            (count, false)
        } else {
            (
                format!("Filter \u{201c}{}\u{201d}: {count} (Esc clears)", b.filter),
                false,
            )
        }
    }
}

/// What the pointer did this frame, held back from the content under an open menu.
type Pointer = ((f32, f32), bool, bool);

fn hide_pointer(ui: &mut Ui) -> Pointer {
    let i = &mut ui.s.input;
    let saved = (i.pos, i.pressed, i.released);
    (i.pos, i.pressed, i.released) = ((-1e6, -1e6), false, false);
    saved
}

fn restore_pointer(ui: &mut Ui, (pos, pressed, released): Pointer) {
    let i = &mut ui.s.input;
    (i.pos, i.pressed, i.released) = (pos, pressed, released);
}

/// "“projects” selected (21 items)" / "(362 KB)".
fn selected(e: &Entry) -> String {
    match (e.dir, e.items) {
        (true, Some(n)) => format!("\u{201c}{}\u{201d} selected ({})", e.name, dir::items(n)),
        (true, None) => format!("\u{201c}{}\u{201d} selected", e.name),
        (false, _) => format!(
            "\u{201c}{}\u{201d} selected ({})",
            e.name,
            dir::human(e.size)
        ),
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
