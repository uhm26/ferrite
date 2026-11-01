//! The components, flat (theme.rs): text helpers, panel, button, icon button, ghost,
//! segmented control, switch, row, navigation entry, meter and scroll
//! thumb. Each draws from the palette, animates hover, press and selection on
//! springs, and reports clicks or edits. Layout is the caller's: widgets get a
//! rect or a spot. No shadows, no outlines (a focused field keeps a ring: field.rs).

use super::ctx::Ui;
use super::icon::Icon;
use super::paint::{Rect, mix};
use super::text::{Ink, MEDIUM, REGULAR, SEMIBOLD};
use super::theme::*;

/// Fully transparent: the "no background" end of a hover mix.
const CLEAR: u32 = 0;
/// A segmented control's inner cell: the control height minus a 2 px rim.
const CELL: f32 = CONTROL_H - 4.0;
/// Shortest scroll thumb, px.
const THUMB_MIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Ui<'_> {
    /// Page title: 20 SemiBold.
    pub fn title_ink(&self) -> Ink {
        Ink::new(TITLE, SEMIBOLD, self.pal.text)
    }

    /// Content text: 14 Regular.
    pub fn body(&self) -> Ink {
        Ink::new(BODY, REGULAR, self.pal.text)
    }

    /// Control and navigation text: 14 Medium.
    pub fn strong(&self) -> Ink {
        Ink::new(BODY, MEDIUM, self.pal.text)
    }

    /// Hints and values: 12 Medium, secondary.
    pub fn small(&self) -> Ink {
        Ink::new(SMALL, MEDIUM, self.pal.sub)
    }

    /// Group headings: 12 SemiBold, secondary.
    pub fn heading(&self) -> Ink {
        Ink::new(SMALL, SEMIBOLD, self.pal.sub)
    }

    /// Text with its baseline at `at`; returns the x after it.
    pub fn label(&mut self, at: (f32, f32), s: &str, ink: Ink) -> f32 {
        self.s.text.draw(&mut self.p, at, s, ink)
    }

    /// `s` in `r`: centered by cap height, aligned, cut with `…` to fit.
    pub fn text(&mut self, r: Rect, s: &str, ink: Ink, align: Align) {
        let s = self.s.text.ellipsize(s, r.w, ink);
        let w = self.s.text.width(&s, ink);
        let x = match align {
            Align::Left => r.x,
            Align::Center => r.x + ((r.w - w) / 2.0).round(),
            Align::Right => (r.x + r.w - w).round(),
        };
        let y = self.s.text.center(r.y, r.h, ink);
        self.s.text.draw(&mut self.p, (x, y), &s, ink);
    }

    /// A group's surface: one flat step above the page.
    pub fn panel(&mut self, r: Rect) {
        let c = self.pal.panel;
        self.p.rect(r.round(), R_PANEL, c);
    }

    /// Click, hover and press of a clickable `r`: (clicked, hover 0-1, press 0-1).
    fn feel(&mut self, name: &str, r: Rect, enabled: bool) -> (bool, f32, f32) {
        let clicked = enabled && self.clicked(name, r);
        let over = enabled && self.over(r);
        let held = enabled && self.held(name);
        let hover = self.spring(&format!("{name}/hover"), f32::from(over), SNAP);
        let press = self.spring(&format!("{name}/press"), f32::from(held), SNAP);
        (clicked, hover, press)
    }

    /// A text button. `primary` is the one emphasis: inverted, dark on light.
    pub fn button(&mut self, name: &str, r: Rect, text: &str, primary: bool) -> bool {
        let r = r.round();
        let (clicked, hover, press) = self.feel(name, r, true);
        let pal = self.pal;
        let (bg, ink) = if primary {
            let bg = mix(
                mix(pal.text, BRIGHT_WHITE, hover * 0.7),
                pal.sub,
                press * 0.4,
            );
            (bg, self.strong().color(WINDOW))
        } else {
            let bg = mix(mix(pal.raise, pal.hover, hover), pal.selected, press);
            (bg, self.strong())
        };
        self.p.rect(r, RADIUS, bg);
        self.text(r.inset(S4), text, ink, Align::Center);
        clicked
    }

    /// A square icon-only button: clear at rest, a surface on hover. Disabled is
    /// dim and takes no clicks.
    pub fn icon_button(&mut self, name: &str, r: Rect, icon: Icon, enabled: bool) -> bool {
        let r = r.round();
        let (clicked, hover, press) = self.feel(name, r, enabled);
        let pal = self.pal;
        let bg = mix(mix(CLEAR, pal.raise, hover), pal.hover, press);
        self.p.rect(r, RADIUS, bg);
        let color = if enabled {
            mix(pal.sub, pal.text, hover)
        } else {
            pal.dim
        };
        self.icon(icon, r, ICON, color);
        clicked
    }

    /// A text-only button: clear at rest, a surface on hover. `current` reads as
    /// primary text; others stay secondary until hovered (path crumbs, links).
    pub fn ghost(&mut self, name: &str, r: Rect, text: &str, current: bool) -> bool {
        let r = r.round();
        let (clicked, hover, press) = self.feel(name, r, true);
        let pal = self.pal;
        let bg = mix(mix(CLEAR, pal.raise, hover), pal.hover, press);
        self.p.rect(r, RADIUS, bg);
        let color = if current {
            pal.text
        } else {
            mix(pal.sub, pal.text, hover)
        };
        let ink = self.strong().color(color);
        self.text(
            Rect::new(r.x + S8, r.y, r.w - 2.0 * S8, r.h),
            text,
            ink,
            Align::Left,
        );
        clicked
    }

    /// Width of a [`Ui::ghost`] showing `text`.
    pub fn ghost_w(&mut self, text: &str) -> f32 {
        let ink = self.strong();
        (self.s.text.width(text, ink) + 2.0 * S8).ceil()
    }

    /// Width of a segmented control of `n` icons.
    pub fn segment_w(n: usize) -> f32 {
        4.0 + n as f32 * CELL
    }

    /// Icons side by side, one chosen; the choice glides between them. Returns
    /// whether `chosen` changed.
    pub fn segment(
        &mut self,
        name: &str,
        at: (f32, f32),
        icons: &[Icon],
        chosen: &mut usize,
    ) -> bool {
        let r = Rect::new(at.0, at.1, Self::segment_w(icons.len()), CONTROL_H).round();
        let pal = self.pal;
        self.p.rect(r, RADIUS, pal.raise);
        let cell = |i: f32| Rect::new(r.x + 2.0 + i * CELL, r.y + 2.0, CELL, CELL);
        let pos = self.spring(&format!("{name}/pos"), *chosen as f32, SNAP);
        self.p.rect(cell(pos).round(), RADIUS - 2.0, pal.selected);
        let mut changed = false;
        for (i, &icon) in icons.iter().enumerate() {
            let c = cell(i as f32);
            let (clicked, hover, _) = self.feel(&format!("{name}/{i}"), c, true);
            if clicked && *chosen != i {
                *chosen = i;
                changed = true;
            }
            let on = (1.0 - (pos - i as f32).abs()).max(hover);
            self.icon(icon, c, ICON, mix(pal.sub, pal.text, on));
        }
        changed
    }

    /// An on/off switch at `at` (top-left); flips `on` when clicked. On is the
    /// inverted emphasis (light track, dark knob).
    pub fn switch(&mut self, name: &str, at: (f32, f32), on: &mut bool) -> bool {
        let r = Rect::new(at.0, at.1, SWITCH_W, SWITCH_H).round();
        let (clicked, hover, _) = self.feel(name, r, true);
        if clicked {
            *on = !*on;
        }
        let t = self.spring(name, f32::from(*on), SNAP);
        let pal = self.pal;
        let track = mix(mix(pal.raise, pal.hover, hover), pal.text, t);
        self.p.rect(r, r.h / 2.0, track);
        let gap = (r.h - KNOB) / 2.0;
        let x = r.x + gap + t * (r.w - 2.0 * gap - KNOB);
        let knob = Rect::new(x, r.y + gap, KNOB, KNOB);
        self.p.rect(knob, KNOB / 2.0, mix(pal.sub, WINDOW, t));
        clicked
    }

    /// A row's background: clear, a surface on hover, `selected` strongest. The
    /// caller draws the content; `ui.spring(name, selected, SNAP)` gives the
    /// selection's 0-1 for fading text in step. True if clicked.
    pub fn row(&mut self, name: &str, r: Rect, selected: bool) -> bool {
        let r = r.round();
        let (clicked, hover, _) = self.feel(name, r, true);
        let sel = self.spring(name, f32::from(selected), SNAP);
        let pal = self.pal;
        let bg = mix(mix(CLEAR, pal.raise, hover), pal.selected, sel);
        self.p.rect(r, RADIUS, bg);
        clicked
    }

    /// The current item's surface, gliding to `to` instead of jumping between items
    /// (GLIDE), fading out where it was when `to` is None. Draw it before the items.
    pub fn slide(&mut self, name: &str, to: Option<Rect>, color: u32) {
        let key = super::ctx::id(name);
        if let Some(r) = to {
            self.s.slides.insert(key, r.round());
        }
        let Some(&r) = self.s.slides.get(&key) else {
            return;
        };
        let shown = self.spring(&format!("{name}/shown"), f32::from(to.is_some()), SNAP);
        let x = self.spring(&format!("{name}/x"), r.x, GLIDE);
        let y = self.spring(&format!("{name}/y"), r.y, GLIDE);
        let w = self.spring(&format!("{name}/w"), r.w, GLIDE);
        let h = self.spring(&format!("{name}/h"), r.h, GLIDE);
        if shown > 0.0 {
            self.p.rect(
                Rect::new(x, y, w, h).round(),
                RADIUS,
                mix(CLEAR, color, shown),
            );
        }
    }

    /// A navigation entry: icon and name, secondary until hovered or current.
    /// An empty `label` draws the icon alone, centered (narrow sidebar). Pair it with
    /// [`Ui::slide`] for the current entry's surface.
    pub fn nav(&mut self, name: &str, r: Rect, (icon, label): (Icon, &str), current: bool) -> bool {
        let r = r.round();
        let (clicked, hover, _) = self.feel(name, r, true);
        let cur = self.spring(name, f32::from(current), SNAP);
        let pal = self.pal;
        // The current entry's surface is the caller's `slide`; this is hover only.
        let bg = mix(CLEAR, pal.raise, hover * 0.6 * (1.0 - cur));
        self.p.rect(r, RADIUS, bg);
        let color = mix(pal.sub, pal.text, cur.max(hover));
        if label.is_empty() {
            self.icon(icon, r, ICON, color);
            return clicked;
        }
        let ib = Rect::new(r.x + S8, r.y, ICON, r.h);
        self.icon(icon, ib, ICON, color);
        let tx = ib.x + ICON + S8;
        let ink = self.strong().color(color);
        self.text(
            Rect::new(tx, r.y, r.x + r.w - S8 - tx, r.h),
            label,
            ink,
            Align::Left,
        );
        clicked
    }

    /// A thin bar showing `frac` (0-1) of a whole: absolute, never rescaled.
    pub fn meter(&mut self, r: Rect, frac: f32) {
        let r = r.round();
        let pal = self.pal;
        self.p.rect(r, r.h / 2.0, pal.raise);
        let fill = Rect {
            w: (r.w * frac.clamp(0.0, 1.0)).round().max(r.h),
            ..r
        };
        self.p.rect(fill, r.h / 2.0, pal.sub);
    }

    /// A scroll thumb along the right edge of `track` for a view `view` px tall over
    /// `content` px scrolled by `offset`. Shows while `active` (pointer over the
    /// scrolled area), fades out after.
    pub fn scroll_thumb(
        &mut self,
        name: &str,
        track: Rect,
        (offset, view, content): (f32, f32, f32),
        active: bool,
    ) {
        let show = self.spring(name, f32::from(active && content > view), GLIDE);
        if show <= 0.0 || content <= view {
            return;
        }
        let h = (view / content * track.h).max(THUMB_MIN).min(track.h);
        let y = track.y + offset / (content - view) * (track.h - h);
        let r = Rect::new(track.x + track.w - THUMB_W, y, THUMB_W, h).round();
        // `dim`, not a surface gray: it must show over selected rows too.
        let c = mix(CLEAR, self.pal.dim, show);
        self.p.rect(r, THUMB_W / 2.0, c);
    }
}

/// The hover end of the primary button: pure white, the only place it's used.
const BRIGHT_WHITE: u32 = 0xffffffff;
