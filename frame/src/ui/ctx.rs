//! Immediate-mode core. The app keeps one [`State`] (springs, focus, input, text);
//! each frame it calls [`State::begin`] for a [`Ui`], draws widgets with it, then
//! [`Ui::end`]. Widgets are found again across frames by an id from their name.

use std::collections::HashMap;

use super::field::Edit;
use super::icon::Icon;
use super::motion::{Spring, Tween};
use super::paint::{Painter, Rect};
use super::text::Text;
use super::theme::Palette;
use crate::window::Canvas;
use crate::xkb::Key;

/// Pointer and keys since the last frame. The app fills it from window events.
#[derive(Default)]
pub struct Input {
    pub pos: (f32, f32),
    pub down: bool,
    /// The left button went down / up since the last frame.
    pub pressed: bool,
    pub released: bool,
    pub keys: Vec<Key>,
    /// Modifiers held now (the app copies them from `Event::Mods`): Ctrl/Shift+click.
    pub mods: crate::xkb::Mods,
}

impl Input {
    pub fn button(&mut self, down: bool) {
        self.down = down;
        if down {
            self.pressed = true;
        } else {
            self.released = true;
        }
    }
}

pub struct State {
    pub text: Text,
    pub input: Input,
    pub glass: bool,
    /// Jump instead of animating (ui skill §3: `reduced_motion`).
    pub reduced: bool,
    /// The widget that has the keyboard.
    pub focus: Option<u64>,
    /// Last frame something hovered was clickable: show the hand cursor.
    pub pointer: bool,
    /// Hold-to-fill progress per button (0-1; below 0: fired, waiting for release).
    pub(crate) holds: HashMap<u64, f32>,
    /// Icon masks per (icon, px size).
    icons: HashMap<(Icon, u32), Box<[u8]>>,
    /// Last target of each `slide`, so it can fade out where it was.
    pub(crate) slides: HashMap<u64, Rect>,
    springs: HashMap<u64, (Spring, u64)>,
    frame: u64,
    dt: f32,
    moving: bool,
    /// The widget the button went down on.
    pub(crate) active: Option<u64>,
    pub(crate) focus_taken: bool,
    /// Caret and selection of the focused field.
    pub(crate) edit: Edit,
    /// Last frame the pointer was over a text field: show the I-beam cursor.
    pub text_cursor: bool,
    /// The field Return was pressed in this frame.
    pub(crate) submitted: Option<u64>,
}

/// A stable id for a widget name (FNV-1a).
pub fn id(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

impl State {
    pub fn new(text: Text) -> Self {
        State {
            text,
            input: Input::default(),
            glass: false,
            reduced: false,
            focus: None,
            pointer: false,
            icons: HashMap::new(),
            holds: HashMap::new(),
            slides: HashMap::new(),
            springs: HashMap::new(),
            frame: 0,
            dt: 0.0,
            moving: false,
            active: None,
            focus_taken: false,
            edit: Edit::END,
            text_cursor: false,
            submitted: None,
        }
    }

    /// Start a frame `dt` seconds after the last one (0 after idle: nothing jumps).
    pub fn begin<'a>(&'a mut self, c: Canvas<'a>, dt: f32) -> Ui<'a> {
        self.frame += 1;
        (self.dt, self.moving, self.pointer, self.focus_taken) = (dt, false, false, false);
        self.text_cursor = false;
        Ui {
            p: Painter::new(c),
            pal: Palette::new(self.glass),
            s: self,
        }
    }

    /// Something moved last frame: draw again on the next frame callback.
    pub fn animating(&self) -> bool {
        self.moving
    }

    /// Springs that exist (for the debug view).
    pub fn springs(&self) -> usize {
        self.springs.len()
    }
}

pub struct Ui<'a> {
    pub p: Painter<'a>,
    pub s: &'a mut State,
    pub pal: Palette,
}

impl Ui<'_> {
    /// The value of spring `name` heading for `target`, advanced once per frame.
    pub fn spring(&mut self, name: &str, target: f32, duration: f32) -> f32 {
        self.spring_from(name, target, target, duration)
    }

    /// [`Ui::spring`], but a spring that doesn't exist yet starts at `from` (so a
    /// surface can grow in instead of popping in). [`Ui::forget`] restarts it.
    pub fn spring_from(&mut self, name: &str, from: f32, target: f32, duration: f32) -> f32 {
        let (frame, dt, reduced) = (self.s.frame, self.s.dt, self.s.reduced);
        let (sp, last) = self
            .s
            .springs
            .entry(id(name))
            .or_insert((Spring::new(from, duration), frame));
        sp.set(target);
        if reduced {
            sp.finish();
        }
        if *last != frame {
            *last = frame;
            sp.step(dt);
        }
        self.s.moving |= sp.moving();
        sp.value
    }

    /// Seconds since the last frame (0 after idle).
    pub fn dt(&self) -> f32 {
        self.s.dt
    }

    /// Drop spring `name`: next use starts fresh.
    pub fn forget(&mut self, name: &str) {
        self.s.springs.remove(&id(name));
    }

    /// Advance an app-owned tween toward `to` by this frame's dt.
    pub fn tween(&mut self, t: &mut Tween, to: f32) -> f32 {
        t.set(to);
        if self.s.reduced {
            t.finish();
        }
        t.step(self.s.dt);
        self.s.moving |= t.moving();
        t.value()
    }

    /// Mark the hand cursor wanted; true if the pointer is over `r`.
    pub fn hovered(&mut self, r: Rect) -> bool {
        let over = self.over(r);
        self.s.pointer |= over;
        over
    }

    /// The pointer is over the visible (unclipped) part of `r`.
    pub fn over(&self, r: Rect) -> bool {
        let pos = self.s.input.pos;
        r.contains(pos) && self.p.in_clip(pos)
    }

    /// A press that started and ended on `r` (named `name`).
    pub fn clicked(&mut self, name: &str, r: Rect) -> bool {
        let (over, i) = (self.hovered(r), &self.s.input);
        if over && i.pressed {
            self.s.active = Some(id(name));
        }
        over && i.released && self.s.active == Some(id(name))
    }

    /// The button is held on `name`.
    pub fn held(&self, name: &str) -> bool {
        self.s.input.down && self.s.active == Some(id(name))
    }

    /// `name` takes the keyboard (a click on a field).
    pub fn take_focus(&mut self, name: &str) {
        self.s.focus = Some(id(name));
        self.s.focus_taken = true;
        self.s.edit = Edit::END;
        // A fresh focus starts its caret and selection where they are, no slide from
        // wherever the last edit left them (field.rs springs).
        for part in ["off", "a", "b", "c"] {
            self.forget(&format!("{name}/{part}"));
        }
    }

    /// Ask for another frame: something changed while drawing that the next frame
    /// must show (no event will come to trigger it).
    pub fn redraw(&mut self) {
        self.s.moving = true;
    }

    /// `name` takes the keyboard with its whole text selected (e.g. a path bar
    /// opened for editing).
    pub fn take_focus_all(&mut self, name: &str) {
        self.take_focus(name);
        self.s.edit = Edit::ALL;
    }

    /// `name` takes the keyboard with bytes `start..end` of its text selected (rename
    /// selects a file's name without its extension).
    pub fn take_focus_range(&mut self, name: &str, start: usize, end: usize) {
        self.take_focus(name);
        self.s.edit = Edit {
            caret: end,
            anchor: start,
            off: 0.0,
        };
    }

    /// Return was pressed in field `name` this frame (the field already let go).
    pub fn submitted(&self, name: &str) -> bool {
        self.s.submitted == Some(id(name))
    }

    pub fn focused(&self, name: &str) -> bool {
        self.s.focus == Some(id(name))
    }

    /// End the frame: a click that no field took drops focus; input is consumed.
    pub fn end(self) {
        let i = &mut self.s.input;
        if i.pressed && !self.s.focus_taken {
            self.s.focus = None;
        }
        self.s.submitted = None;
        if i.released {
            self.s.active = None;
        }
        (i.pressed, i.released) = (false, false);
        i.keys.clear();
    }
}

impl Ui<'_> {
    /// `icon` at `size` px, centered in `r`, on whole px.
    pub fn icon(&mut self, icon: Icon, r: Rect, size: f32, color: u32) {
        let px = size.round() as usize;
        let mask = self
            .s
            .icons
            .entry((icon, px as u32))
            .or_insert_with(|| icon.mask(px));
        let x = (r.x + (r.w - size) / 2.0).round() as i32;
        let y = (r.y + (r.h - size) / 2.0).round() as i32;
        self.p.coverage((x, y), (px, px), mask, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::FAMILY;

    /// A widget scrolled half under a clip edge only takes clicks on its visible part.
    #[test]
    fn clicks_ignore_the_clipped_part() {
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut px = vec![0u32; 100 * 100];
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut hit = |s: &mut State, pos| {
            s.input.pos = pos;
            s.input.button(true);
            s.input.button(false);
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 100,
                    h: 100,
                },
                0.0,
            );
            ui.p.clip(Some(Rect::new(0.0, 50.0, 100.0, 50.0)));
            let h = ui.clicked("w", r);
            ui.end();
            h
        };
        assert!(!hit(&mut s, (10.0, 10.0)), "clipped part took the click");
        assert!(hit(&mut s, (10.0, 60.0)));
    }

    /// A field opened with everything selected: typing replaces, Return submits.
    #[test]
    fn field_select_all_replaces_and_return_submits() {
        use crate::xkb::{Key, Mods};
        let key = |sym: u32, text: &str| Key {
            sym,
            text: text.into(),
            mods: Mods::default(),
            repeat: false,
        };
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut px = vec![0u32; 200 * 40];
        let mut value = String::from("/old/path");
        let r = Rect::new(0.0, 0.0, 200.0, 32.0);
        let mut frame = |s: &mut State, keys: Vec<Key>, value: &mut String, open: bool| -> bool {
            s.input.keys = keys;
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 200,
                    h: 40,
                },
                0.0,
            );
            if open {
                ui.take_focus_all("path");
            }
            ui.field("path", r, value, "");
            let done = ui.submitted("path");
            ui.end();
            done
        };
        frame(&mut s, vec![], &mut value, true);
        frame(
            &mut s,
            vec![key(0x2f, "/"), key(0x74, "t")],
            &mut value,
            false,
        );
        assert_eq!(value, "/t", "typing replaced the selected text");
        assert!(frame(&mut s, vec![key(0xff0d, "\r")], &mut value, false));
        assert!(s.focus.is_none());
    }

    /// `slide` glides to a new target over a few frames instead of jumping.
    #[test]
    fn slide_glides_between_targets() {
        let mut s = State::new(Text::new(FAMILY).unwrap());
        let mut px = vec![0u32; 120 * 20];
        let mut frame = |s: &mut State, x: f32, dt: f32| -> Vec<u32> {
            px.fill(0);
            let mut ui = s.begin(
                Canvas {
                    px: &mut px,
                    w: 120,
                    h: 20,
                },
                dt,
            );
            ui.slide("sel", Some(Rect::new(x, 0.0, 16.0, 16.0)), 0xffff_ffff);
            ui.end();
            px.clone()
        };
        let lit = |img: &[u32], x: usize| img[8 * 120 + x] == 0xffff_ffff;
        let start = frame(&mut s, 0.0, 0.0);
        assert!(lit(&start, 8));
        let mid = frame(&mut s, 100.0, 0.05);
        assert!(!lit(&mid, 108), "jumped straight to the target");
        assert!((9..100).any(|x| lit(&mid, x)), "did not move at all");
        let mut end = mid;
        for _ in 0..20 {
            end = frame(&mut s, 100.0, 0.05);
        }
        assert!(lit(&end, 108) && !lit(&end, 8));
        // A size change (the layout resizing) is followed at once, no lag.
        let mut ui = s.begin(Canvas { px: &mut px, w: 120, h: 20 }, 0.05);
        ui.slide("sel", Some(Rect::new(100.0, 0.0, 20.0, 16.0)), 0xffff_ffff);
        ui.end();
        assert!(lit(&px, 118), "the wider surface lagged");
    }
}
