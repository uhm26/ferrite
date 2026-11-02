//! The gallery's frame, the app shell every frame app uses (`frame::ui::shell`): a
//! folding, resizable sidebar of pages, a toolbar, a status line, and the page in the
//! rounded pane as a title plus groups of rows on flat panels in one centered column.
//! Helpers here build the groups and rows.

use frame::ui::theme::*;
use frame::ui::{Align, Icon, Rect, SideItem, Sidebar, Ui};

/// The sidebar folds to icons in windows narrower than this.
const NARROW_BELOW: f32 = 640.0;
/// The widest the column of groups gets.
const CONTENT_MAX: f32 = 680.0;
/// One row in a group: a control plus 8 above and below.
pub const ROW: f32 = CONTROL_H + 2.0 * S8;
/// Title line height.
const TITLE_H: f32 = 32.0;
/// Heading line above a group.
const HEADING_H: f32 = 28.0;
/// The sidebar footer: one line of small text.
const FOOTER_H: f32 = 20.0;

/// Icon, name and sidebar section of each page.
pub const PAGES: [(Icon, &str, &str); 6] = [
    (Icon::Check, "Controls", "Components"),
    (Icon::List, "Lists", "Components"),
    (Icon::Documents, "Type", "Look"),
    (Icon::Image, "Color", "Look"),
    (Icon::Grid, "Icons", "Look"),
    (Icon::Forward, "Motion", "Look"),
];

/// A sidebar that folds below [`NARROW_BELOW`].
pub fn sidebar() -> Sidebar {
    let mut side = Sidebar::default();
    side.content_min = NARROW_BELOW - side.width;
    side
}

/// The pages column; switches `page` on a click and returns its width this frame
/// (it glides). The footer shows where the page is in the list.
pub fn nav(ui: &mut Ui, side: &mut Sidebar, page: &mut usize) -> f32 {
    let items = PAGES.map(|(icon, label, section)| SideItem {
        icon,
        label,
        section,
    });
    let n = *page;
    let mut footer = |ui: &mut Ui, r: Rect, a: f32| {
        let small = ui.small();
        let small = small.color(frame::ui::paint::mix(0, small.color, a));
        let text = format!("Page {} of {}", n + 1, PAGES.len());
        ui.text(r, &text, small, Align::Left);
    };
    let (w, clicked) = side.draw(ui, &items, Some(*page), Some((FOOTER_H, &mut footer)));
    if let Some(i) = clicked {
        *page = i;
    }
    w
}

/// The page column inside `pane`: x and width, centered, at most `CONTENT_MAX` wide.
pub fn column(pane: Rect) -> (f32, f32) {
    let room = (pane.w - 2.0 * S24).max(0.0);
    let w = room.min(CONTENT_MAX);
    ((pane.x + S24 + (room - w) / 2.0).round(), w.round())
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
