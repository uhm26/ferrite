//! The window's content: Nemo's layout in frame's flat look. Places run down the
//! left in frame's app shell (folding to icons in narrow windows); the toolbar has back, forward, up,
//! the path (or a filter) and the view switch; the entries sit in a rounded pane as
//! a list or a grid; a status bar closes the frame. Right-click opens a menu of
//! actions (actions.rs); keys are in keys.rs.

mod actions;
mod info;
mod items;
mod keys;
mod location;
mod menu;
mod pick;
mod sidebar;
mod tabs;
mod toolbar;

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use frame::ui::paint::mix;
use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, Rect, Shell, Ui};
use frame::xkb::Mods;

use crate::browse::Browser;
use crate::browse::search::MAX_RESULTS;
use crate::browse::tabs::Tabs;
use crate::dir::{self, Entry, Kind};
use crate::ops::trash;
use crate::ops::{Jobs, Undo};
use crate::thumbs::Thumbs;
use actions::{Clip, Menu, Rename, Transfer};
use items::drag::Drags;
use items::grid::Grid;
use items::list::{self, List};
use items::scroll::Scroll;
use location::Location;
use menu::OpenWith;
use sidebar::Places;
use tabs::TabReq;

pub use pick::Pick;
pub use sidebar::home;

/// GTK's default double-click time.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// The view switch: list or grid.
const LIST: usize = 0;
/// The Empty Trash button: width, and how long it must be held.
const EMPTY_W: f32 = 176.0;
const HOLD_SECS: f32 = 1.0;

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
    /// The selection box, and files dragged out of or into the window.
    drags: Drags,
    /// Text for the system clipboard (a copied path).
    copy_text: Option<String>,
    /// Files in ~/Templates, looked up when a menu opens.
    templates: Vec<PathBuf>,
    /// Apps for the file a menu opened on.
    open_with: Option<OpenWith>,
    /// The rounded pane the entries sit in (last frame): overlays center on it.
    pane: Rect,
    /// A paste or drop asking what to do with names already taken (and how many).
    prompt: Option<(Transfer, usize)>,
    /// Finished operations, newest last (Ctrl+Z takes them back).
    undo: Vec<Undo>,
    /// Folder sizes the info pane asked for: bytes and files.
    sizes: HashMap<PathBuf, (u64, u64)>,
    /// Folders being measured right now (asked once each).
    measuring: HashSet<PathBuf>,
    /// The info pane is open (Ctrl+I), and what it read about the file shown.
    info_open: bool,
    details: Option<info::Details>,
    /// Tabs on the strip (last frame) and their folders; changes asked for while
    /// drawing; a middle click waiting for the next frame.
    tab_rects: Vec<(Rect, PathBuf)>,
    tab_reqs: Vec<TabReq>,
    middle_click: Option<(f32, f32)>,
    /// Split view: the second pane's state, and where drops into its folder land.
    side: items::panes::Side,
    /// A tab pressed or dragged on the strip.
    tab_drag: Option<tabs::TabDrag>,
    other_drop: Option<(Rect, PathBuf)>,
    /// Where files dragged over us would land (a folder outline or a line between
    /// entries), worked out before drawing.
    drop_hint: Option<Rect>,
    /// Picker mode (the portal), and what was picked.
    pick: Option<Pick>,
    picked: Option<Vec<PathBuf>>,
    /// Pictures shrunk for the grid.
    thumbs: Thumbs,
}

impl View {
    pub fn new() -> io::Result<Self> {
        Ok(View {
            quit: false,
            places: Places::new()?,
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
            drags: Drags::default(),
            templates: Vec::new(),
            open_with: None,
            undo: Vec::new(),
            prompt: None,
            pane: Rect::default(),
            sizes: HashMap::new(),
            measuring: HashSet::new(),
            info_open: false,
            details: None,
            tab_rects: Vec::new(),
            tab_reqs: Vec::new(),
            middle_click: None,
            side: items::panes::Side::default(),
            tab_drag: None,
            other_drop: None,
            drop_hint: None,
            pick: None,
            picked: None,
            thumbs: Thumbs::new()?,
        })
    }

    /// Open the info pane (headless shots).
    pub fn set_info(&mut self, on: bool) {
        self.info_open = on;
    }

    /// Show the grid instead of the list.
    pub fn set_grid(&mut self, on: bool, zoom: usize) {
        self.mode = usize::from(on);
        self.grid.zoom = 0;
        for _ in 0..zoom {
            self.grid.zoom_by(1);
        }
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
        self.places.side.resizing()
    }

    /// The pipe finished file jobs report through (the app polls it).
    pub fn jobs_fd(&self) -> BorrowedFd<'_> {
        self.jobs.fd()
    }

    /// The pipe finished thumbnails wake the loop through.
    pub fn thumbs_fd(&self) -> BorrowedFd<'_> {
        self.thumbs.fd()
    }

    /// The pipe the drives worker wakes the loop through.
    pub fn drives_fd(&self) -> BorrowedFd<'_> {
        self.places.drives.fd()
    }

    /// Take in drive changes (and go into a drive just mounted).
    pub fn drive_reports(&mut self, b: &mut Browser) {
        if let Some(n) = self.places.drive_reports(b) {
            self.note = Some(n);
        }
    }

    /// Take in finished thumbnails. True if any (redraw).
    pub fn thumbs_ready(&mut self) -> bool {
        self.thumbs.drain()
    }

    /// The right button went down at `pos`: a menu opens on the next frame.
    pub fn right_click(&mut self, pos: (f32, f32)) {
        self.right_click = Some(pos);
    }

    /// The wheel: scrolls, or with Ctrl zooms the grid (mouse wheel notches; a
    /// touchpad only scrolls).
    pub fn scroll_by(&mut self, discrete: i32, px: f64, ctrl: bool) {
        if ctrl && self.mode != LIST {
            self.grid.zoom_by(-discrete);
        } else if self.menu.is_none() {
            self.scroll.wheel(discrete, px);
        }
    }

    /// The middle button went down at `pos`: handled on the next frame.
    pub fn middle_click(&mut self, pos: (f32, f32)) {
        self.middle_click = Some(pos);
    }

    /// A field, the menu or a prompt has the keyboard.
    fn keyboard_taken(&self) -> bool {
        self.loc.open || self.rename.is_some() || self.menu.is_some() || self.prompt.is_some()
    }

    pub fn draw(&mut self, ui: &mut Ui, tabs: &mut Tabs) {
        // A press in the split view's other pane makes it the pane in use first.
        let pressed = self.focus_on_press(ui, tabs);
        self.follow_panes(tabs.pair_ids());
        if let Some(at) = pressed {
            self.select_pressed(ui, tabs.cur_mut(), at);
        }
        self.tab_keys(ui, tabs);
        if let Some(at) = self.middle_click.take().filter(|_| !self.keyboard_taken()) {
            self.middle(tabs, at);
        }
        self.drop_hint = self.drop_target(tabs, ui.s.input.pos).map(|t| t.0).filter(|_| self.drags.over);
        let lines = tabs::lines(tabs);
        let (cur, other_i, left) = (tabs.index(), tabs.other(), tabs.other_left());
        let (b, other) = tabs.pair_mut();
        self.draw_tab(ui, b, ((&lines, cur, other_i), other.map(|o| (o, left))));
        if self.apply_tab_reqs(tabs) {
            ui.redraw();
        }
    }

    fn draw_tab(&mut self, ui: &mut Ui, b: &mut Browser, strip: Panes) {
        let (gen0, open0) = (b.generation, self.loc.open);
        self.keys(ui, b);
        // While the menu is open the content below must not see the pointer.
        let overlay = self.menu.is_some() || self.prompt.is_some();
        let pointer = overlay.then(|| hide_pointer(ui));
        self.frame(ui, b, strip);
        if let Some(saved) = pointer {
            restore_pointer(ui, saved);
        }
        if let Some(at) = self.right_click.take() {
            let on = self.item_at(at, b.entries.len());
            self.open_menu(ui, at, on, b);
        }
        let name = self.item_rects(b.sel).map(|r| r.1);
        if self.rename.is_some() {
            self.rename(ui, b, name);
        }
        self.menu(ui, b);
        let pane = self.pane;
        self.ask(ui, b, pane);
        // Changed after parts were drawn (a click opened a folder, a bar came or
        // went): draw again, no event will ask for it.
        if b.generation != gen0 || self.loc.open != open0 {
            ui.redraw();
        }
    }

    /// Everything but the menu and the rename field.
    fn frame(&mut self, ui: &mut Ui, b: &mut Browser, (strip, other): Panes) {
        let w = ui.p.size().0;
        // The sidebar folds before the list would lose its Type and Modified columns.
        self.places.side.content_min = S8 + 2.0 * S4 + list::ALL_COLS_W;
        let nav_w = self.places.draw(ui, b);
        // The info pane glides in on the right; the list makes room for it.
        let info_w = ui
            .spring(
                "info/open",
                if self.info_open { info::INFO_W } else { 0.0 },
                GLIDE,
            )
            .round();
        let shell = Shell::new(ui, nav_w, info_w);
        let pane = shell.pane;
        self.pane = pane;
        self.toolbar(ui, b, shell.toolbar);
        self.status(ui, b, shell.status_halves());
        let panel = self.tab_strip(ui, pane.inset(S4), strip);
        let (panel, side_r) = self.panes(ui, (w, panel), other.as_ref().map(|o| o.1));
        self.page = if self.mode == LIST {
            ((panel.h / ROW_H) as usize).saturating_sub(2)
        } else {
            self.grid.cols * ((panel.h / self.grid.tile_h()) as usize).saturating_sub(1)
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
            self.grid
                .draw(ui, b, (panel, &mut self.thumbs), &mut self.scroll)
        };
        if let Some(by) = self.list.sort_click.take() {
            b.sort_by(by);
        }
        if let Some(i) = clicked {
            let mods = ui.s.input.mods;
            self.click(b, i, mods);
        }
        self.other_drop = None;
        if let (Some((ob, _)), Some(r)) = (other, side_r) {
            self.other_pane(ui, ob, r);
        }
        self.gestures(ui, b, panel);
        if info_w >= 1.0 {
            self.info(ui, b, shell.aside);
        }
    }

    /// Click selects; a second click on the same entry soon after opens it.
    /// Ctrl adds or removes it, Shift selects the range to it (no double-click then).
    fn click(&mut self, b: &mut Browser, i: usize, mods: Mods) {
        b.click(i, (mods.ctrl, mods.shift));
        let double = !mods.ctrl
            && !mods.shift
            && self
                .last_click
                .is_some_and(|(j, t)| j == i && t.elapsed() < DOUBLE_CLICK);
        if double {
            self.last_click = None;
            self.open_entry(b);
            self.scroll.follow();
        } else {
            self.last_click = Some((i, Instant::now()));
        }
    }

    /// The status bar, like Nemo's: on the left what just happened, the find text,
    /// the filter or the item count (errors in red); on the right the selection.
    fn status(&self, ui: &mut Ui, b: &Browser, (half, right): (Rect, Rect)) {
        let (left, red) = self.status_text(b);
        let small = Ink::new(SMALL, REGULAR, if red { ERROR } else { ui.pal.sub });
        if !self.progress(ui, half) {
            ui.text(half, &left, small, Align::Left);
        }
        if let Some(what) = selected_text(b) {
            let small = ui.small();
            ui.text(right, &what, small, Align::Right);
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
        if let Some(s) = &b.search {
            let n = b.entries.len();
            let text = if s.recent {
                format!("Recent: {} (Esc goes back)", dir::items(n))
            } else if b.searching() {
                format!("Searching \u{201c}{}\u{201d}\u{2026} {n} found", s.text)
            } else {
                let cap = if n >= MAX_RESULTS {
                    ", stopped there"
                } else {
                    ""
                };
                format!(
                    "\u{201c}{}\u{201d}: {n} found in subfolders{cap} (Esc goes back)",
                    s.text
                )
            };
            return (text, false);
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

/// The tab strip's lines and the current tab; the other pane's browser and whether
/// it sits on the left (split view).
type Panes<'a> = (
    tabs::Strip<'a>,
    Option<(&'a mut Browser, bool)>,
);

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

/// "“projects” selected (21 items)" / "(362 KB)", or "3 items selected (1.2 MB)".
fn selected_text(b: &Browser) -> Option<String> {
    let sel = b.selection();
    if sel.len() > 1 {
        let bytes: u64 = sel.iter().map(|&i| b.entries[i].size).sum();
        return Some(format!(
            "{} items selected ({})",
            sel.len(),
            dir::human(bytes)
        ));
    }
    b.entries.get(b.sel).map(selected)
}

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

/// Width of the progress meter in the status bar.
const PROGRESS_W: f32 = 120.0;

impl View {
    /// A running copy or move in `r`: meter, "Copying 3 items 45 %", and × to cancel.
    /// False when nothing is running.
    fn progress(&self, ui: &mut Ui, r: Rect) -> bool {
        let Some((label, frac)) = self.jobs.progress() else {
            return false;
        };
        let bar = Rect::new(r.x, r.y + (r.h - S4) / 2.0, PROGRESS_W, S4);
        ui.meter(bar, frac);
        let text = format!("{label} {:.0} %", frac * 100.0);
        let tx = bar.x + bar.w + S12;
        let small = ui.small();
        let tw = ui.s.text.width(&text, small).ceil();
        ui.text(Rect::new(tx, r.y, tw, r.h), &text, small, Align::Left);
        let close = Rect::new(tx + tw + S4, r.y + (r.h - 24.0) / 2.0, 24.0, 24.0);
        if ui.icon_button("progress/cancel", close, Icon::Close, true) {
            self.jobs.cancel();
        }
        true
    }

    /// Something is running in the background (the loop redraws for its progress).
    pub fn busy(&self) -> bool {
        self.jobs.running > 0
    }
}

impl View {
    /// The toolbar in `bar`, and the Empty Trash button when the Trash has items.
    fn toolbar(&mut self, ui: &mut Ui, b: &mut Browser, bar: Rect) {
        let empty = b.cwd == trash::files_dir() && !b.entries.is_empty() && self.pick.is_none();
        let reserve = if empty { EMPTY_W + S12 } else { self.pick_w() };
        if toolbar::draw(ui, b, (bar, reserve), &mut self.mode, &mut self.loc) {
            self.scroll.follow();
        }
        if let Some(done) = self.loc.renamed.take() {
            self.note = Some(format!("Renamed {}", dir::items(done.len())));
            self.undo.push(Undo::Moved(done));
        }
        if empty {
            let x = bar.x + bar.w - Ui::segment_w(2) - S12 - EMPTY_W;
            let r = Rect::new(x, bar.y, EMPTY_W, CONTROL_H);
            if ui.hold_button("empty-trash", r, "Hold to Empty Trash", HOLD_SECS) {
                self.empty_trash(true);
            }
        }
        // The picker's controls end where the view switch starts.
        let right = bar.x + bar.w - Ui::segment_w(2) - S12;
        self.pick_bar(ui, b, (bar, right));
    }
}
