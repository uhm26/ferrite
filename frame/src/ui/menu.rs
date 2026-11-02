//! A context menu: one surface that grows out of the point it opened at (its height
//! springs open, nothing pops in), items with an optional key hint on the right,
//! separators, disabled items dimmed, and submenus (an item with children shows a
//! chevron; hovering it or → opens them beside it, ← goes back). ↑/↓ move, Return
//! picks, Esc closes; a press outside closes. The app owns whether it is open, where,
//! and what it holds; while it is open the app keeps the pointer from what's below.

use super::ctx::Ui;
use super::icon::Icon;
use super::paint::{Rect, mix};
use super::text::{Ink, MEDIUM};
use super::theme::*;
use super::widgets::Align;

const KEY_RETURN: u32 = 0xff0d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_LEFT: u32 = 0xff51;
const KEY_UP: u32 = 0xff52;
const KEY_RIGHT: u32 = 0xff53;
const KEY_DOWN: u32 = 0xff54;
/// A separator's height: a hairline with 4 px above and below.
const SEP_H: f32 = S8 + 1.0;
const MIN_W: f32 = 200.0;
/// Space between an item's label and its hint.
const HINT_GAP: f32 = S32;
/// The chevron marking an item with a submenu.
const CHEVRON: f32 = 12.0;

#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    /// Empty: a separator.
    pub label: &'a str,
    /// A key hint shown dim on the right ("Ctrl+C"); may be empty.
    pub hint: &'a str,
    pub enabled: bool,
    /// Children: this item opens a submenu instead of being picked.
    pub sub: &'a [MenuItem<'a>],
}

impl<'a> MenuItem<'a> {
    pub const SEPARATOR: MenuItem<'static> = MenuItem {
        label: "",
        hint: "",
        enabled: false,
        sub: &[],
    };

    pub const fn new(label: &'a str, hint: &'a str, enabled: bool) -> Self {
        MenuItem {
            label,
            hint,
            enabled,
            sub: &[],
        }
    }

    /// An item opening `sub` (disabled when `sub` is empty).
    pub const fn submenu(label: &'a str, sub: &'a [MenuItem<'a>]) -> Self {
        MenuItem {
            label,
            hint: "",
            enabled: !sub.is_empty(),
            sub,
        }
    }

    fn pickable(&self) -> bool {
        self.enabled && !self.label.is_empty()
    }
}

/// The menu's state between frames: highlighted items, which submenu is drawn (it
/// stays while it shrinks away), whether the whole menu is closing, and the last
/// pointer position (for the safe triangle).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MenuHot {
    pub top: Option<usize>,
    pub sub: Option<usize>,
    shown_sub: Option<usize>,
    closing: bool,
    last_pos: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    Open,
    /// The item picked, and which child of it when it has a submenu. The menu then
    /// shrinks away by itself: keep calling until `Closed`.
    Picked(usize, Option<usize>),
    /// Fully gone (after shrinking): stop drawing it.
    Closed,
}

impl Ui<'_> {
    /// A menu opened at `at`. Call [`Ui::forget`]`("{name}/grow")` and reset `hot`
    /// when opening it anew so it grows in again. Closing (Esc, a press outside, a
    /// pick) shrinks it first; `Closed` comes once it is gone.
    pub fn menu(
        &mut self,
        name: &str,
        at: (f32, f32),
        items: &[MenuItem],
        hot: &mut MenuHot,
    ) -> MenuEvent {
        let r = self.menu_rect(at, items);
        let pos = self.s.input.pos;
        let mut picked = None;
        if !hot.closing {
            match self.menu_keys(items, hot) {
                Some(MenuEvent::Picked(i, j)) => picked = Some((i, j)),
                Some(_) => hot.closing = true,
                None => {}
            }
            let in_sub = hot
                .shown_sub
                .is_some_and(|i| self.sub_rect(r, items, i).contains(pos));
            if self.s.input.pressed && !r.contains(pos) && !in_sub {
                hot.closing = true;
            }
        }
        let grow = self.spring_from(&format!("{name}/grow"), 0.0, f32::from(!hot.closing), SNAP);
        if hot.closing && grow <= 0.02 {
            self.forget(&format!("{name}/grow"));
            *hot = MenuHot::default();
            return MenuEvent::Closed;
        }
        self.surface(r, r.y + r.h <= at.1 + 1.0, grow);
        // Heading for the submenu, the rows crossed on the way don't take the highlight.
        let heading = hot
            .shown_sub
            .is_some_and(|i| toward(hot.last_pos, pos, self.sub_rect(r, items, i)));
        let live = !hot.closing && picked.is_none() && !heading;
        let before = hot.top;
        let clicked = self.menu_items(name, r, (items, live), &mut hot.top);
        if hot.top != before {
            hot.sub = None;
        }
        self.p.clip(None);
        if let Some(i) = clicked.filter(|&i| items[i].sub.is_empty()) {
            picked = Some((i, None));
        }
        if let Some(j) = self.submenu(name, (r, items), hot) {
            picked = Some((hot.top.unwrap_or(0), Some(j)));
        }
        hot.last_pos = pos;
        match picked {
            Some((i, j)) => {
                hot.closing = true;
                MenuEvent::Picked(i, j)
            }
            None => MenuEvent::Open,
        }
    }

    /// The submenu of the highlighted item: grows in when it opens, shrinks away when
    /// the highlight leaves it (or the menu closes). Returns the child clicked.
    fn submenu(
        &mut self,
        name: &str,
        (r, items): (Rect, &[MenuItem]),
        hot: &mut MenuHot,
    ) -> Option<usize> {
        let open = hot
            .top
            .filter(|&i| !hot.closing && !items[i].sub.is_empty());
        if let Some(new) = open.filter(|&o| hot.shown_sub != Some(o)) {
            // Both reset: the old one goes at once, the new one grows in from 0.
            for i in hot.shown_sub.into_iter().chain([new]) {
                self.forget(&format!("{name}/sub{i}/grow"));
            }
            hot.shown_sub = open;
        }
        let i = hot.shown_sub?;
        let key = format!("{name}/sub{i}/grow");
        let g = self.spring_from(&key, 0.0, f32::from(open == Some(i)), SNAP);
        if open != Some(i) && g <= 0.02 {
            self.forget(&key);
            hot.shown_sub = None;
            return None;
        }
        let s = self.sub_rect(r, items, i);
        self.surface(s, false, g);
        let live = open == Some(i);
        let clicked = self.menu_items(
            &format!("{name}/sub{i}"),
            s,
            (items[i].sub, live),
            &mut hot.sub,
        );
        self.p.clip(None);
        clicked
    }

    /// A menu surface `r` shown `grow` (0-1) of its height, from its bottom when `up`.
    /// Leaves the clip on it for the rows.
    fn surface(&mut self, r: Rect, up: bool, grow: f32) {
        let h = (r.h * grow).round();
        let shown = Rect {
            y: if up { r.y + r.h - h } else { r.y },
            h,
            ..r
        };
        let pal = self.pal;
        self.p.rect(shown, R_PANEL, pal.raise);
        self.p.clip(Some(shown));
    }

    /// Keys: ↑/↓ in the active level, → into a submenu, ← or Esc out of it, Return picks
    /// (or enters a submenu), Esc at the top closes.
    fn menu_keys(&mut self, items: &[MenuItem], hot: &mut MenuHot) -> Option<MenuEvent> {
        let open = hot.top.filter(|&i| !items[i].sub.is_empty());
        for k in std::mem::take(&mut self.s.input.keys) {
            let sub = open.map(|i| items[i].sub);
            match (k.sym, hot.sub, sub) {
                (KEY_ESCAPE | KEY_LEFT, Some(_), _) => hot.sub = None,
                (KEY_ESCAPE, None, _) => return Some(MenuEvent::Closed),
                (KEY_RIGHT | KEY_RETURN, None, Some(s)) => hot.sub = step(s, None, true),
                (KEY_RETURN, Some(j), Some(s)) if s[j].pickable() => {
                    return Some(MenuEvent::Picked(open.unwrap_or(0), Some(j)));
                }
                (KEY_RETURN, None, None) if hot.top.is_some_and(|i| items[i].pickable()) => {
                    return Some(MenuEvent::Picked(hot.top.unwrap_or(0), None));
                }
                (KEY_DOWN | KEY_UP, Some(_), Some(s)) => {
                    hot.sub = step(s, hot.sub, k.sym == KEY_DOWN)
                }
                (KEY_DOWN | KEY_UP, ..) => {
                    hot.top = step(items, hot.top, k.sym == KEY_DOWN);
                    hot.sub = None;
                }
                _ => {}
            }
        }
        None
    }

    /// Width and height of a menu holding `items`.
    fn menu_size(&mut self, items: &[MenuItem]) -> (f32, f32) {
        let (label, hint) = (self.strong(), self.small());
        let w = items
            .iter()
            .map(|i| {
                let hw = if !i.sub.is_empty() {
                    HINT_GAP + CHEVRON
                } else if i.hint.is_empty() {
                    0.0
                } else {
                    HINT_GAP + self.s.text.width(i.hint, hint)
                };
                self.s.text.width(i.label, label) + hw
            })
            .fold(0.0, f32::max);
        let h: f32 = items
            .iter()
            .map(|i| if i.label.is_empty() { SEP_H } else { CONTROL_H })
            .sum();
        ((w + 2.0 * S12 + 2.0 * S4).max(MIN_W).ceil(), h + 2.0 * S4)
    }

    /// Size and place the menu: at `at`, kept inside the window (flipped above or to
    /// the left when it would run off).
    fn menu_rect(&mut self, at: (f32, f32), items: &[MenuItem]) -> Rect {
        let (w, h) = self.menu_size(items);
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

    /// Where item `i`'s submenu goes: beside its row (left of the menu if there is no
    /// room on the right), its first row level with it, kept inside the window.
    fn sub_rect(&mut self, r: Rect, items: &[MenuItem], i: usize) -> Rect {
        let (w, h) = self.menu_size(items[i].sub);
        let row_y = r.y
            + S4
            + items[..i]
                .iter()
                .map(|it| {
                    if it.label.is_empty() {
                        SEP_H
                    } else {
                        CONTROL_H
                    }
                })
                .sum::<f32>();
        let (win_w, win_h) = self.p.size();
        let right = r.x + r.w + 2.0;
        let x = if right + w > win_w - S4 {
            r.x - w - 2.0
        } else {
            right
        };
        let y = (row_y - S4).min(win_h - S4 - h).max(S4);
        Rect::new(x.max(S4), y, w, h).round()
    }

    /// The rows; while `live`, hover moves `hot` and a click on an enabled item picks
    /// it (a closing menu or one the pointer is crossing only draws).
    fn menu_items(
        &mut self,
        name: &str,
        r: Rect,
        (items, live): (&[MenuItem], bool),
        hot: &mut Option<usize>,
    ) -> Option<usize> {
        let pal = self.pal;
        let mut y = r.y + S4;
        let mut picked = None;
        for (i, it) in items.iter().enumerate() {
            if it.label.is_empty() {
                let line = Rect::new(r.x + S12, y + S4, r.w - 2.0 * S12, 1.0);
                // `selected`, not `line`: a line gray vanishes on the raised surface.
                self.p.rect(line, 0.0, pal.selected);
                y += SEP_H;
                continue;
            }
            let row = Rect::new(r.x + S4, y, r.w - 2.0 * S4, CONTROL_H);
            y += CONTROL_H;
            if live && it.enabled && self.over(row) {
                *hot = Some(i);
            }
            if live && it.enabled && self.clicked(&format!("{name}/{i}"), row) {
                picked = Some(i);
            }
            let lit = self.spring(&format!("{name}/{i}/hot"), f32::from(*hot == Some(i)), SNAP);
            self.p.rect(row, RADIUS, mix(0, pal.selected, lit));
            let color = if it.enabled { pal.text } else { pal.dim };
            let inner = Rect::new(row.x + S8, row.y, row.w - 2.0 * S8, row.h);
            self.text(inner, it.label, Ink::new(BODY, MEDIUM, color), Align::Left);
            if it.sub.is_empty() {
                let hint = self
                    .small()
                    .color(if it.enabled { pal.sub } else { pal.dim });
                self.text(inner, it.hint, hint, Align::Right);
            } else {
                let chev = Rect::new(inner.x + inner.w - CHEVRON, row.y, CHEVRON, row.h);
                self.icon(
                    Icon::Forward,
                    chev,
                    CHEVRON,
                    if it.enabled { pal.sub } else { pal.dim },
                );
            }
        }
        picked
    }
}

/// The pointer moved from `last` to `pos` into the triangle between `last` and the
/// submenu `s`'s near edge: it is heading for the submenu.
fn toward(last: (f32, f32), pos: (f32, f32), s: Rect) -> bool {
    if last == pos || s.contains(pos) {
        return s.contains(pos);
    }
    let x = if s.x > last.0 { s.x } else { s.x + s.w };
    in_triangle(pos, last, (x, s.y), (x, s.y + s.h))
}

fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |u: (f32, f32), v: (f32, f32)| (v.0 - u.0) * (p.1 - u.1) - (v.1 - u.1) * (p.0 - u.0);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
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

    /// One menu frame per entry of `frames` (its keys), then `tail` empty frames; a
    /// press outside on frame `press` if given. Returns the events and the last hot.
    fn run(
        items: &[MenuItem],
        frames: &[Vec<u32>],
        (tail, press): (usize, Option<usize>),
    ) -> (Vec<MenuEvent>, MenuHot) {
        let key = |sym| Key {
            sym,
            text: String::new(),
            mods: Mods::default(),
            repeat: false,
        };
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut px = vec![0u32; 600 * 400];
        let mut hot = MenuHot::default();
        let mut out = Vec::new();
        let all = frames
            .iter()
            .cloned()
            .chain(std::iter::repeat_n(Vec::new(), tail));
        for (n, keys) in all.enumerate() {
            s.input.keys = keys.iter().map(|&k| key(k)).collect();
            if press == Some(n) {
                s.input.pos = (590.0, 390.0);
                s.input.button(true);
            }
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 600,
                    h: 400,
                },
                0.016,
            );
            out.push(ui.menu("m", (10.0, 10.0), items, &mut hot));
            ui.end();
        }
        (out, hot)
    }

    #[test]
    fn return_picks_then_it_shrinks_away() {
        let items = [
            MenuItem::new("Open", "", true),
            MenuItem::new("Copy", "", true),
        ];
        let (ev, _) = run(
            &items,
            &[vec![], vec![KEY_DOWN, KEY_DOWN], vec![KEY_RETURN]],
            (40, None),
        );
        assert_eq!(ev[2], MenuEvent::Picked(1, None));
        assert_eq!(ev[3], MenuEvent::Open, "still shrinking");
        assert_eq!(ev.iter().filter(|e| **e == MenuEvent::Closed).count(), 1);
    }

    #[test]
    fn a_press_outside_closes_after_shrinking() {
        let items = [MenuItem::new("Open", "", true)];
        // Pressed once it is fully open (frame 20), it shrinks before it is gone.
        let (ev, _) = run(&items, &vec![vec![]; 21], (40, Some(20)));
        assert_eq!(ev[20], MenuEvent::Open, "shrinks first");
        assert!(ev.contains(&MenuEvent::Closed));
    }

    #[test]
    fn submenus_open_pick_and_shrink_when_left() {
        let sub = [
            MenuItem::new("Text", "", true),
            MenuItem::new("Markdown", "", true),
        ];
        let items = [
            MenuItem::new("Paste", "", true),
            MenuItem::submenu("New", &sub),
        ];
        let frames = [
            vec![KEY_DOWN, KEY_DOWN],
            vec![KEY_RIGHT, KEY_DOWN],
            vec![KEY_RETURN],
        ];
        let (ev, _) = run(&items, &frames, (0, None));
        assert_eq!(ev[2], MenuEvent::Picked(1, Some(1)));
        // Highlight moves off "New": its submenu stays a few frames (shrinking), then goes.
        let (_, hot) = run(
            &items,
            &[vec![KEY_DOWN, KEY_DOWN], vec![], vec![KEY_UP]],
            (0, None),
        );
        assert_eq!(hot.shown_sub, Some(1), "shrinking, not gone at once");
        let (_, hot) = run(
            &items,
            &[vec![KEY_DOWN, KEY_DOWN], vec![], vec![KEY_UP]],
            (40, None),
        );
        assert_eq!(hot.shown_sub, None);
    }

    #[test]
    fn heading_into_the_submenu_is_recognized() {
        let s = Rect::new(200.0, 50.0, 150.0, 120.0);
        assert!(
            toward((100.0, 60.0), (130.0, 70.0), s),
            "diagonal toward it"
        );
        assert!(!toward((100.0, 60.0), (90.0, 90.0), s), "moving away");
        assert!(toward((100.0, 60.0), (210.0, 60.0), s), "inside it");
    }
}
