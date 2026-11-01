//! The gallery's frame, laid out the way frame apps are meant to look: a
//! full-height navigation column, a page title, and groups of rows on flat panels
//! in one centered column. Narrow windows fold the navigation to icons; the
//! crossing glides (resize.md). Helpers here build the groups and rows.

use frame::ui::theme::*;
use frame::ui::{Align, Icon, Rect, Ui};

/// Navigation column, open and folded to icons.
const NAV_W: f32 = 200.0;
const NAV_NARROW_W: f32 = CONTROL_H + 2.0 * S8;
/// Below this window width the navigation shows icons only.
const NAV_NARROW_BELOW: f32 = 640.0;
/// The widest the column of groups gets.
const CONTENT_MAX: f32 = 680.0;
/// One row in a group: a control plus 8 above and below.
pub const ROW: f32 = CONTROL_H + 2.0 * S8;
/// Title line height.
const TITLE_H: f32 = 32.0;
/// Heading line above a group.
const HEADING_H: f32 = 28.0;

pub const PAGES: [(Icon, &str); 6] = [
    (Icon::Check, "Controls"),
    (Icon::List, "Lists"),
    (Icon::Documents, "Type"),
    (Icon::Image, "Color"),
    (Icon::Grid, "Icons"),
    (Icon::Forward, "Motion"),
];

/// The navigation column; returns its width this frame (it glides).
pub fn nav(ui: &mut Ui, page: &mut usize) -> f32 {
    let (w, h) = ui.p.size();
    let open = ui.spring("nav/open", f32::from(w >= NAV_NARROW_BELOW), GLIDE);
    let nav_w = (NAV_NARROW_W + (NAV_W - NAV_NARROW_W) * open).round();
    let side = ui.pal.sidebar;
    ui.p.rect(Rect::new(0.0, 0.0, nav_w, h), 0.0, side);
    ui.p.clip(Some(Rect::new(0.0, 0.0, nav_w, h)));
    let item = |i: usize| {
        Rect::new(
            S8,
            S16 + i as f32 * (CONTROL_H + 2.0),
            nav_w - 2.0 * S8,
            CONTROL_H,
        )
    };
    let raise = ui.pal.raise;
    ui.slide("nav/current", Some(item(*page)), raise);
    for (i, (icon, name)) in PAGES.into_iter().enumerate() {
        let r = item(i);
        // Names go once the column is too narrow for them (cut at the edge, never squashed).
        let label = if open > 0.5 { name } else { "" };
        if ui.nav(&format!("nav/{i}"), r, (icon, label), *page == i) {
            *page = i;
        }
    }
    ui.p.clip(None);
    nav_w
}

/// The page column right of the navigation: x and width, centered, at most
/// `CONTENT_MAX` wide.
pub fn column(win_w: f32, nav_w: f32) -> (f32, f32) {
    let room = (win_w - nav_w - 2.0 * S24).max(0.0);
    let w = room.min(CONTENT_MAX);
    ((nav_w + S24 + (room - w) / 2.0).round(), w.round())
}

/// Lays a page out top to bottom: title, then groups.
pub struct Flow {
    pub x: f32,
    pub w: f32,
    pub y: f32,
}

impl Flow {
    pub fn title(&mut self, ui: &mut Ui, text: &str) {
        let ink = ui.title_ink();
        ui.text(
            Rect::new(self.x, self.y, self.w, TITLE_H),
            text,
            ink,
            Align::Left,
        );
        self.y += TITLE_H + S16;
    }

    /// A heading and a panel holding `rows` rows of `row_h`; returns the row rects.
    pub fn group(&mut self, ui: &mut Ui, heading: &str, (rows, row_h): (usize, f32)) -> Vec<Rect> {
        let ink = ui.heading();
        ui.text(
            Rect::new(self.x + S4, self.y, self.w, HEADING_H),
            heading,
            ink,
            Align::Left,
        );
        self.y += HEADING_H;
        let panel = Rect::new(self.x, self.y, self.w, rows as f32 * row_h + 2.0 * S4);
        ui.panel(panel);
        let out = (0..rows)
            .map(|i| {
                Rect::new(
                    self.x + S4,
                    self.y + S4 + i as f32 * row_h,
                    self.w - 2.0 * S4,
                    row_h,
                )
            })
            .collect();
        self.y += panel.h + S24;
        out
    }

    /// A bare panel `h` tall (for free layouts); returns its inside, `S16` in.
    pub fn block(&mut self, ui: &mut Ui, heading: &str, h: f32) -> Rect {
        let ink = ui.heading();
        ui.text(
            Rect::new(self.x + S4, self.y, self.w, HEADING_H),
            heading,
            ink,
            Align::Left,
        );
        self.y += HEADING_H;
        let panel = Rect::new(self.x, self.y, self.w, h);
        ui.panel(panel);
        self.y += h + S24;
        panel.inset(S16)
    }
}

/// A settings row: name (and a hint under it) on the left. Returns where the
/// control goes: the row's right part, vertically centered, `w` wide.
pub fn setting(ui: &mut Ui, r: Rect, (name, hint): (&str, &str), w: f32) -> Rect {
    let text_w = r.w - w - 3.0 * S12;
    let strong = ui.strong();
    if hint.is_empty() {
        ui.text(
            Rect::new(r.x + S12, r.y, text_w, r.h),
            name,
            strong,
            Align::Left,
        );
    } else {
        // Two lines centered as a pair: name 18 px, hint 16 px.
        let top = r.y + (r.h - 34.0) / 2.0;
        ui.text(
            Rect::new(r.x + S12, top, text_w, 18.0),
            name,
            strong,
            Align::Left,
        );
        let small = ui.small();
        ui.text(
            Rect::new(r.x + S12, top + 18.0, text_w, 16.0),
            hint,
            small,
            Align::Left,
        );
    }
    Rect::new(
        r.x + r.w - S12 - w,
        r.y + (r.h - CONTROL_H) / 2.0,
        w,
        CONTROL_H,
    )
}
