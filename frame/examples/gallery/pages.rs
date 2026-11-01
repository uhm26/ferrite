//! The gallery's pages: controls, lists, type, color, icons and motion, each a
//! title plus groups (layout.rs). Wheel scrolls a page; keys switch pages.

use frame::ui::motion::Tween;
use frame::ui::text::{BOLD, MEDIUM, REGULAR, SEMIBOLD};
use frame::ui::theme::*;
use frame::ui::{Align, Icon, Ink, MenuEvent, MenuItem, Rect, Ui};

use crate::debug::Stats;
use crate::layout::{self, Flow, PAGES, ROW};

const DRAWER_H: f32 = 240.0;
/// xkbcommon keysyms.
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_UP: u32 = 0xff52;
const KEY_DOWN: u32 = 0xff54;
const KEY_F12: u32 = 0xffc9;

/// Rows for the list demo: icon, name, size.
const FILES: [(Icon, &str, &str); 6] = [
    (Icon::Folder, "projects", ""),
    (Icon::Folder, "Downloads", ""),
    (Icon::Documents, "notes.md", "2.9 KB"),
    (Icon::Image, "wallpaper.png", "4.1 MB"),
    (
        Icon::Music,
        "a-very-long-file-name-that-gets-cut-with-an-ellipsis.flac",
        "31 MB",
    ),
    (Icon::File, "Cargo.lock", "12 KB"),
];

pub struct Gallery {
    pub quit: bool,
    pub debug: bool,
    pub page: usize,
    scroll: f32,
    /// Bottom of the page content last frame, for the scroll limit.
    content_h: f32,
    name: String,
    row: usize,
    nav_row: usize,
    view: usize,
    clicks: u32,
    moved: bool,
    drawer_open: bool,
    drawer: Tween,
    /// The demo menu: where it is open and its highlighted item.
    menu: Option<(f32, f32)>,
    menu_hot: Option<usize>,
}

impl Gallery {
    pub fn new() -> Self {
        Gallery {
            quit: false,
            debug: false,
            page: 0,
            scroll: 0.0,
            content_h: 0.0,
            name: String::new(),
            row: 2,
            nav_row: 0,
            view: 0,
            clicks: 0,
            moved: false,
            drawer_open: false,
            drawer: Tween::new(0.0, DRAWER, DRAWER_CURVE),
            menu: None,
            menu_hot: None,
        }
    }

    pub fn scroll_by(&mut self, discrete: i32, px: f64) {
        self.scroll += if discrete != 0 {
            discrete as f32 * WHEEL_STEP
        } else {
            px as f32
        };
    }

    pub fn draw(&mut self, ui: &mut Ui, stats: &Stats) {
        let (w, h) = ui.p.size();
        let window = ui.pal.window;
        ui.p.clear(window);
        let shown = self.page;
        let nav_w = layout::nav(ui, &mut self.page);
        if self.page != shown {
            self.scroll = 0.0;
        }
        let (x, cw) = layout::column(w, nav_w);
        self.scroll = self.scroll.clamp(0.0, (self.content_h - h).max(0.0));
        // One spring per page: switching pages starts at that page's offset.
        let off = ui
            .spring(&format!("scroll/{}", self.page), self.scroll, GLIDE)
            .round();
        let area = Rect::new(nav_w, 0.0, w - nav_w, h);
        ui.p.clip(Some(area));
        let mut f = Flow {
            x,
            w: cw,
            y: S24 - off,
        };
        f.title(ui, PAGES[self.page].1);
        match self.page {
            0 => self.controls(ui, &mut f),
            1 => self.lists(ui, &mut f),
            2 => type_page(ui, &mut f),
            3 => color_page(ui, &mut f),
            4 => icons_page(ui, &mut f),
            _ => self.motion(ui, &mut f),
        }
        ui.p.clip(None);
        self.content_h = f.y + off;
        let over = area.contains(ui.s.input.pos);
        ui.scroll_thumb(
            "scroll/thumb",
            area.inset(S4),
            (off, h, self.content_h),
            over,
        );
        self.drawer(ui, (nav_w, w), h);
        self.menu(ui);
        if self.debug {
            stats.draw(ui, w);
        }
        self.keys(ui);
    }

    /// Shortcuts, for keys no focused field took.
    fn keys(&mut self, ui: &mut Ui) {
        for k in std::mem::take(&mut ui.s.input.keys) {
            match (k.sym, k.text.as_str()) {
                (KEY_ESCAPE, _) => self.quit = true,
                (KEY_F12, _) => self.debug = !self.debug,
                (KEY_UP, _) => self.page = self.page.saturating_sub(1),
                (KEY_DOWN, _) => self.page = (self.page + 1).min(PAGES.len() - 1),
                (_, "g") => ui.s.glass = !ui.s.glass,
                (_, "m") => ui.s.reduced = !ui.s.reduced,
                _ => {}
            }
        }
    }

    fn controls(&mut self, ui: &mut Ui, f: &mut Flow) {
        let rows = f.group(ui, "Buttons", (4, ROW));
        let c = layout::setting(ui, rows[0], ("Primary", "The one emphasis on a page"), 96.0);
        self.clicks += u32::from(ui.button("btn/primary", c, "Save", true));
        let c = layout::setting(
            ui,
            rows[1],
            ("Secondary", &format!("{} clicks", self.clicks)),
            96.0,
        );
        self.clicks += u32::from(ui.button("btn/secondary", c, "Cancel", false));
        let c = layout::setting(
            ui,
            rows[2],
            ("Icon buttons", "Forward is disabled"),
            3.0 * CONTROL_H + S8,
        );
        for (i, (icon, on)) in [
            (Icon::Back, true),
            (Icon::Forward, false),
            (Icon::Search, true),
        ]
        .into_iter()
        .enumerate()
        {
            let r = Rect::new(c.x + i as f32 * (CONTROL_H + S4), c.y, CONTROL_H, CONTROL_H);
            self.clicks += u32::from(ui.icon_button(&format!("ib/{i}"), r, icon, on));
        }
        let c = layout::setting(
            ui,
            rows[3],
            ("Segment", "The choice glides"),
            Ui::segment_w(2),
        );
        ui.segment(
            "seg/view",
            (c.x, c.y),
            &[Icon::List, Icon::Grid],
            &mut self.view,
        );

        self.switches(ui, f);
    }

    fn switches(&mut self, ui: &mut Ui, f: &mut Flow) {
        let rows = f.group(ui, "Switches", (3, ROW));
        let (mut glass, mut reduced) = (ui.s.glass, ui.s.reduced);
        let switches = [
            (
                "sw/glass",
                ("Glass", "See-through surfaces (G)"),
                &mut glass,
            ),
            (
                "sw/reduced",
                ("Reduced motion", "Jump instead of animating (M)"),
                &mut reduced,
            ),
            (
                "sw/debug",
                ("Debug overlay", "Frame time, memory, springs (F12)"),
                &mut self.debug,
            ),
        ];
        for (r, (name, text, on)) in rows.into_iter().zip(switches) {
            let c = layout::setting(ui, r, text, SWITCH_W);
            ui.switch(name, (c.x, c.y + (CONTROL_H - SWITCH_H) / 2.0), on);
        }
        (ui.s.glass, ui.s.reduced) = (glass, reduced);

        let rows = f.group(ui, "Text and menus", (2, ROW));
        let hint = format!("{} chars; Esc or Return lets go", self.name.chars().count());
        let c = layout::setting(ui, rows[0], ("Name", &hint), 240.0);
        ui.field("field/name", c, &mut self.name, "Type here…");
        let c = layout::setting(ui, rows[1], ("Menu", "Grows from where it opens"), 96.0);
        if ui.button("btn/menu", c, "Open", false) {
            ui.forget("demo-menu/grow");
            (self.menu, self.menu_hot) = (Some((c.x, c.y + c.h + S4)), None);
        }
    }

    fn lists(&mut self, ui: &mut Ui, f: &mut Flow) {
        let rows = f.group(ui, "Rows", (FILES.len(), ROW_H));
        for (i, (r, (icon, name, size))) in rows.into_iter().zip(FILES).enumerate() {
            let id = format!("row/{i}");
            if ui.row(&id, r, self.row == i) {
                self.row = i;
            }
            let sel = ui.spring(&id, f32::from(self.row == i), SNAP);
            let sub = frame::ui::paint::mix(ui.pal.sub, ui.pal.text, sel);
            ui.icon(icon, Rect::new(r.x + S8, r.y, ICON, r.h), ICON, sub);
            let size_w = 72.0;
            let nx = r.x + S8 + ICON + S8;
            let body = ui.body();
            ui.text(
                Rect::new(nx, r.y, r.w - (nx - r.x) - size_w - S12, r.h),
                name,
                body,
                Align::Left,
            );
            let small = ui.small().color(sub);
            ui.text(
                Rect::new(r.x + r.w - S12 - size_w, r.y, size_w, r.h),
                size,
                small,
                Align::Right,
            );
        }

        let places = [
            (Icon::Home, "Home"),
            (Icon::Documents, "Documents"),
            (Icon::Download, "Downloads"),
        ];
        let rows = f.group(ui, "Navigation", (places.len(), CONTROL_H + 2.0));
        let raise = ui.pal.raise;
        ui.slide(
            "place/current",
            Some(Rect {
                h: CONTROL_H,
                ..rows[self.nav_row]
            }),
            raise,
        );
        for (i, (r, place)) in rows.into_iter().zip(places).enumerate() {
            let r = Rect { h: CONTROL_H, ..r };
            if ui.nav(&format!("place/{i}"), r, place, self.nav_row == i) {
                self.nav_row = i;
            }
        }

        let rows = f.group(ui, "Meter", (1, ROW));
        let c = layout::setting(ui, rows[0], ("Disk", "349 GB free of 378 GB"), 160.0);
        ui.meter(
            Rect::new(c.x, c.y + (c.h - S4) / 2.0, c.w, S4),
            29.0 / 378.0,
        );
    }

    fn motion(&mut self, ui: &mut Ui, f: &mut Flow) {
        let drawer_t = self.drawer.value();
        let rows = f.group(ui, "Springs and curves", (3, ROW));
        let tracks = [
            ("Snap", "0.15 s spring", SNAP),
            ("Glide", "0.22 s spring", GLIDE),
            ("Drawer", "0.5 s curve", 0.0),
        ];
        for (i, (r, (name, hint, dur))) in rows.into_iter().zip(tracks).enumerate() {
            let c = layout::setting(ui, r, (name, hint), 240.0);
            let track = Rect::new(c.x, c.y + c.h / 2.0 - 2.0, c.w, S4);
            let line = ui.pal.raise;
            ui.p.rect(track, 2.0, line);
            let t = if dur > 0.0 {
                ui.spring(&format!("demo/{i}"), f32::from(self.moved), dur)
            } else {
                drawer_t
            };
            let dot = Rect::new(
                track.x + t * (track.w - 12.0),
                c.y + c.h / 2.0 - 6.0,
                12.0,
                12.0,
            );
            let text = ui.pal.text;
            ui.p.rect(dot.round(), 6.0, text);
        }
        let rows = f.group(ui, "Try it", (2, ROW));
        let c = layout::setting(ui, rows[0], ("Move the dots", ""), 96.0);
        self.moved ^= ui.button("btn/move", c, "Move", false);
        let label = if self.drawer_open { "Close" } else { "Open" };
        let c = layout::setting(
            ui,
            rows[1],
            ("Drawer", "Grows out of the bottom edge"),
            96.0,
        );
        self.drawer_open ^= ui.button("btn/drawer", c, label, true);
    }

    /// A surface that grows out of the bottom edge (height only, never sideways).
    fn drawer(&mut self, ui: &mut Ui, (left, w): (f32, f32), h: f32) {
        let t = ui.tween(&mut self.drawer, f32::from(self.drawer_open));
        let dh = (t * DRAWER_H).round();
        if dh < 1.0 {
            return;
        }
        let (x, cw) = layout::column(w, left);
        // Taller than shown by the radius: its bottom corners stay below the edge.
        let r = Rect::new(x, h - dh, cw, dh + R_PANEL);
        let fill = ui.pal.raise;
        ui.p.rect(r, R_PANEL, fill);
        ui.p.clip(Some(Rect { h: dh, ..r }));
        let ink = ui.strong();
        ui.text(
            Rect::new(r.x + S16, r.y + S12, r.w - 2.0 * S16, 24.0),
            "Drawer",
            ink,
            Align::Left,
        );
        let small = ui.small();
        let note = "500 ms on the graphic curve (0.76, 0, 0.12, 1)";
        ui.text(
            Rect::new(r.x + S16, r.y + 36.0, r.w - 2.0 * S16, 16.0),
            note,
            small,
            Align::Left,
        );
        let row = Rect::new(r.x + S8, r.y + 64.0, r.w - 2.0 * S8, CONTROL_H);
        if ui.nav("drawer/close", row, (Icon::Close, "Close"), false) {
            self.drawer_open = false;
        }
        ui.p.clip(None);
    }
}

fn type_page(ui: &mut Ui, f: &mut Flow) {
    let (text, sub) = (ui.pal.text, ui.pal.sub);
    let lines = [
        (Ink::new(TITLE, SEMIBOLD, text), "Title 20 SemiBold"),
        (
            Ink::new(BODY, REGULAR, text),
            "Body 14 Regular: the quick brown fox 0123",
        ),
        (
            Ink::new(BODY, MEDIUM, text),
            "Strong 14 Medium: controls and navigation",
        ),
        (
            Ink::new(BODY, BOLD, text),
            "Bold 14: rare, for numbers that matter",
        ),
        (
            Ink::new(SMALL, MEDIUM, sub),
            "Small 12 Medium: hints and values",
        ),
        (
            Ink::new(SMALL, SEMIBOLD, sub),
            "Heading 12 SemiBold: above a group",
        ),
        (Ink::new(BODY, REGULAR, text), "Fallback: 漢字 ✓ → ★"),
        (
            Ink::new(BODY, REGULAR, text),
            "Cut to fit: a very long line of text that does not fit in its box gets an ellipsis at the end",
        ),
    ];
    let rows = f.group(ui, "Scale", (lines.len(), 40.0));
    for (r, (ink, s)) in rows.into_iter().zip(lines) {
        ui.text(
            Rect::new(r.x + S12, r.y, r.w - 2.0 * S12, r.h),
            s,
            ink,
            Align::Left,
        );
    }
}

fn color_page(ui: &mut Ui, f: &mut Flow) {
    let surfaces = [
        ("window", WINDOW),
        ("sidebar", SIDEBAR),
        ("panel", PANEL),
        ("raise", RAISE),
        ("hover", HOVER),
        ("selected", SELECTED),
    ];
    swatches(ui, f, "Surfaces, darkest to lightest", &surfaces);
    let inks = [
        ("text", TEXT),
        ("sub", SUB),
        ("dim", DIM),
        ("line", LINE),
        ("accent", ACCENT),
        ("error", ERROR),
    ];
    swatches(ui, f, "Text and marks", &inks);
}

fn swatches(ui: &mut Ui, f: &mut Flow, heading: &str, tokens: &[(&str, u32)]) {
    let cols = 3.0;
    let sw = ((f.w - 2.0 * S16 - (cols - 1.0) * S12) / cols).floor();
    let rows = (tokens.len() as f32 / cols).ceil();
    let inner = f.block(ui, heading, rows * 84.0 - S12 + 2.0 * S16);
    for (i, (name, color)) in tokens.iter().enumerate() {
        let (cx, cy) = ((i % 3) as f32, (i / 3) as f32);
        let s = Rect::new(inner.x + cx * (sw + S12), inner.y + cy * 84.0, sw, 40.0).round();
        ui.p.rect(s, RADIUS, *color);
        // A swatch may match the panel it sits on: outline it so it still shows.
        let line = ui.pal.line;
        ui.p.stroke(s, RADIUS, 1.0, line);
        let strong = ui.strong();
        ui.text(
            Rect::new(s.x + S4, s.y + 44.0, sw, 18.0),
            name,
            strong,
            Align::Left,
        );
        let small = ui.small();
        let hex = format!("#{:06x}", color & 0xff_ffff);
        ui.text(
            Rect::new(s.x + S4, s.y + 60.0, sw, 14.0),
            &hex,
            small,
            Align::Left,
        );
    }
}

fn icons_page(ui: &mut Ui, f: &mut Flow) {
    let (tile_w, tile_h) = (96.0, 76.0);
    let cols = ((f.w - 2.0 * S12) / tile_w).floor().max(1.0);
    let rows = (Icon::ALL.len() as f32 / cols).ceil();
    let inner = f.block(
        ui,
        "16 px, 1.5 px strokes, drawn by frame",
        rows * tile_h + 2.0 * S16 - 2.0 * S4,
    );
    let inner = inner.inset(-S4);
    for (i, icon) in Icon::ALL.into_iter().enumerate() {
        let (cx, cy) = ((i as f32) % cols, (i as f32 / cols).floor());
        let t = Rect::new(inner.x + cx * tile_w, inner.y + cy * tile_h, tile_w, tile_h);
        let text = ui.pal.text;
        ui.icon(icon, Rect { h: 44.0, ..t }, ICON, text);
        let small = ui.small();
        ui.text(
            Rect::new(t.x, t.y + 44.0, t.w, 16.0),
            &format!("{icon:?}"),
            small,
            Align::Center,
        );
    }
}

impl Gallery {
    /// The demo menu, drawn last so it sits on top.
    fn menu(&mut self, ui: &mut Ui) {
        let Some(at) = self.menu else {
            return;
        };
        let items = [
            MenuItem::new("Open", "Return", true),
            MenuItem::SEPARATOR,
            MenuItem::new("Copy", "Ctrl+C", true),
            MenuItem::new("Paste", "Ctrl+V", false),
            MenuItem::SEPARATOR,
            MenuItem::new("Rename…", "F2", true),
        ];
        match ui.menu("demo-menu", at, &items, &mut self.menu_hot) {
            MenuEvent::Open => {}
            MenuEvent::Picked(_) => {
                self.clicks += 1;
                self.menu = None;
            }
            MenuEvent::Closed => self.menu = None,
        }
    }
}
