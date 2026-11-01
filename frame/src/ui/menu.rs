//! A context menu: one surface that grows out of the point it opened at (its height
//! springs open, nothing pops in), items with an optional key hint on the right,
//! separators, disabled items dimmed. ↑/↓ move, Return picks, Esc closes; a press
//! outside closes. The app owns whether it is open, where, and what it holds; while
//! it is open the app keeps the pointer from the content under it.

use super::ctx::Ui;
use super::paint::{Rect, mix};
use super::text::{Ink, MEDIUM};
use super::theme::*;
use super::widgets::Align;

const KEY_RETURN: u32 = 0xff0d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_UP: u32 = 0xff52;
const KEY_DOWN: u32 = 0xff54;
/// A separator's height: a hairline with 4 px above and below.
const SEP_H: f32 = S8 + 1.0;
const MIN_W: f32 = 200.0;
/// Space between an item's label and its hint.
const HINT_GAP: f32 = S32;

#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    /// Empty: a separator.
    pub label: &'a str,
    /// A key hint shown dim on the right ("Ctrl+C"); may be empty.
    pub hint: &'a str,
    pub enabled: bool,
}

impl<'a> MenuItem<'a> {
    pub const SEPARATOR: MenuItem<'static> = MenuItem {
        label: "",
        hint: "",
        enabled: false,
    };

    pub const fn new(label: &'a str, hint: &'a str, enabled: bool) -> Self {
        MenuItem {
            label,
            hint,
            enabled,
        }
    }

    fn pickable(&self) -> bool {
        self.enabled && !self.label.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    Open,
    Picked(usize),
    Closed,
}

impl Ui<'_> {
    /// A menu opened at `at`; `hot` is the highlighted item (keys and hover move it).
    /// Call [`Ui::forget`]`(name)` when opening it anew so it grows in again.
    pub fn menu(
        &mut self,
        name: &str,
        at: (f32, f32),
        items: &[MenuItem],
        hot: &mut Option<usize>,
    ) -> MenuEvent {
        let r = self.menu_rect(at, items);
        let keys = std::mem::take(&mut self.s.input.keys);
        for k in keys {
            match k.sym {
                KEY_ESCAPE => return MenuEvent::Closed,
                KEY_RETURN if hot.is_some_and(|i| items[i].pickable()) => {
                    return MenuEvent::Picked(hot.unwrap_or(0));
                }
                KEY_DOWN => *hot = step(items, *hot, true),
                KEY_UP => *hot = step(items, *hot, false),
                _ => {}
            }
        }
        if self.s.input.pressed && !r.contains(self.s.input.pos) {
            return MenuEvent::Closed;
        }
        // Grows from the edge it opened at: down from a click, up when flipped above.
        let grow = self.spring_from(&format!("{name}/grow"), 0.0, 1.0, SNAP);
        let up = r.y + r.h <= at.1 + 1.0;
        let h = (r.h * grow).round();
        let shown = Rect {
            y: if up { r.y + r.h - h } else { r.y },
            h,
            ..r
        };
        let pal = self.pal;
        self.p.rect(shown, R_PANEL, pal.raise);
        self.p.clip(Some(shown));
        let picked = self.menu_items(name, r, items, hot);
        self.p.clip(None);
        picked.map_or(MenuEvent::Open, MenuEvent::Picked)
    }

    /// Size and place the menu: at `at`, kept inside the window (flipped above or to
    /// the left when it would run off).
    fn menu_rect(&mut self, at: (f32, f32), items: &[MenuItem]) -> Rect {
        let (label, hint) = (self.strong(), self.small());
        let w = items
            .iter()
            .map(|i| {
                let hw = if i.hint.is_empty() {
                    0.0
                } else {
                    HINT_GAP + self.s.text.width(i.hint, hint)
                };
                self.s.text.width(i.label, label) + hw
            })
            .fold(0.0, f32::max);
        let w = (w + 2.0 * S12 + 2.0 * S4).max(MIN_W).ceil();
        let h: f32 = items
            .iter()
            .map(|i| if i.label.is_empty() { SEP_H } else { CONTROL_H })
            .sum::<f32>()
            + 2.0 * S4;
        let (win_w, win_h) = self.p.size();
        let x = if at.0 + w > win_w - S4 {
            at.0 - w
        } else {
            at.0
        };
        let y = if at.1 + h > win_h - S4 {
            at.1 - h
        } else {
            at.1
        };
        Rect::new(x.max(S4), y.max(S4), w, h).round()
    }

    /// The rows; hover moves `hot`, a click on an enabled item picks it.
    fn menu_items(
        &mut self,
        name: &str,
        r: Rect,
        items: &[MenuItem],
        hot: &mut Option<usize>,
    ) -> Option<usize> {
        let pal = self.pal;
        let mut y = r.y + S4;
        let mut picked = None;
        for (i, it) in items.iter().enumerate() {
            if it.label.is_empty() {
                self.p.rect(
                    Rect::new(r.x + S12, y + S4, r.w - 2.0 * S12, 1.0),
                    0.0,
                    // `selected`, not `line`: a line gray vanishes on the raised surface.
                    pal.selected,
                );
                y += SEP_H;
                continue;
            }
            let row = Rect::new(r.x + S4, y, r.w - 2.0 * S4, CONTROL_H);
            y += CONTROL_H;
            if it.enabled && self.over(row) {
                *hot = Some(i);
            }
            if it.enabled && self.clicked(&format!("{name}/{i}"), row) {
                picked = Some(i);
            }
            let lit = self.spring(&format!("{name}/{i}/hot"), f32::from(*hot == Some(i)), SNAP);
            self.p.rect(row, RADIUS, mix(0, pal.selected, lit));
            let color = if it.enabled { pal.text } else { pal.dim };
            let inner = Rect::new(row.x + S8, row.y, row.w - 2.0 * S8, row.h);
            self.text(inner, it.label, Ink::new(BODY, MEDIUM, color), Align::Left);
            let hint = self
                .small()
                .color(if it.enabled { pal.sub } else { pal.dim });
            self.text(inner, it.hint, hint, Align::Right);
        }
        picked
    }
}

/// The next pickable item after (or before) `from`, wrapping around.
fn step(items: &[MenuItem], from: Option<usize>, down: bool) -> Option<usize> {
    let n = items.len();
    let start = from.unwrap_or(if down { n - 1 } else { 0 });
    (1..=n)
        .map(|k| {
            if down {
                (start + k) % n
            } else {
                (start + n - k % n) % n
            }
        })
        .find(|&i| items[i].pickable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{State, Text};
    use crate::window::Canvas;
    use crate::xkb::{Key, Mods};

    #[test]
    fn keys_skip_separators_and_disabled_items() {
        let items = [
            MenuItem::new("Open", "Return", true),
            MenuItem::SEPARATOR,
            MenuItem::new("Paste", "Ctrl+V", false),
            MenuItem::new("Rename", "F2", true),
        ];
        assert_eq!(step(&items, None, true), Some(0));
        assert_eq!(step(&items, Some(0), true), Some(3));
        assert_eq!(step(&items, Some(3), true), Some(0), "wraps");
        assert_eq!(step(&items, Some(0), false), Some(3));
    }

    #[test]
    fn return_picks_and_a_press_outside_closes() {
        let key = |sym| Key {
            sym,
            text: String::new(),
            mods: Mods::default(),
            repeat: false,
        };
        let items = [
            MenuItem::new("Open", "", true),
            MenuItem::new("Copy", "", true),
        ];
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut px = vec![0u32; 400 * 300];
        let mut hot = None;
        let mut frame = |s: &mut State, keys: Vec<Key>, hot: &mut Option<usize>| {
            s.input.keys = keys;
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 400,
                    h: 300,
                },
                0.016,
            );
            let ev = ui.menu("m", (10.0, 10.0), &items, hot);
            ui.end();
            ev
        };
        assert_eq!(frame(&mut s, vec![], &mut hot), MenuEvent::Open);
        frame(&mut s, vec![key(KEY_DOWN), key(KEY_DOWN)], &mut hot);
        assert_eq!(
            frame(&mut s, vec![key(KEY_RETURN)], &mut hot),
            MenuEvent::Picked(1)
        );
        s.input.pos = (390.0, 290.0);
        s.input.button(true);
        assert_eq!(frame(&mut s, vec![], &mut hot), MenuEvent::Closed);
    }
}
