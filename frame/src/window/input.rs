//! Seat input: keyboard (xkb keymap, modifiers, key repeat) and pointer (motion,
//! buttons, wheel). Turns protocol events into window [`Event`]s.
//! Copied into frame from ../term/src/window/input.rs at term 7be66f5 (2026-11-01).

use std::time::{Duration, Instant};

use super::{Event, Window};
use crate::sys::mem::Map;
use crate::wl::proto::{cursor_shape, keyboard, pointer, seat};
use crate::wl::{Arg, Error, Event as WlEvent};
use crate::xkb::{self, Key, Keyboard};

pub enum Pointer {
    /// Position in surface pixels.
    Motion(f64, f64),
    Button {
        button: u32,
        pressed: bool,
    },
    /// Wheel clicks (`discrete`) or smooth scroll pixels (`px`); positive = down.
    Scroll {
        discrete: i32,
        px: f64,
    },
}

pub(super) struct Input {
    keyboard: u32,
    pointer: u32,
    cursor_dev: u32,
    /// Serial of the last pointer enter (cursor changes must quote it) and the
    /// cursor shape set since then (0 = none yet).
    enter_serial: u32,
    shape: u32,
    xkb: Option<Keyboard>,
    /// Repeats per second (0 = off) and the delay before the first repeat.
    rate: u32,
    delay: Duration,
    repeat: Option<(u32, Instant)>,
    /// Latest input serial (needed to set the clipboard).
    pub serial: u32,
    scroll_discrete: i32,
    scroll_px: f64,
}

impl Default for Input {
    fn default() -> Self {
        Input {
            keyboard: 0,
            pointer: 0,
            cursor_dev: 0,
            enter_serial: 0,
            shape: 0,
            xkb: None,
            rate: 25,
            delay: Duration::from_millis(600),
            repeat: None,
            serial: 0,
            scroll_discrete: 0,
            scroll_px: 0.0,
        }
    }
}

impl Input {
    pub fn repeat_deadline(&self) -> Option<Instant> {
        self.repeat.map(|(_, at)| at)
    }

    pub fn repeat_due(&mut self, now: Instant) -> Option<Key> {
        let (key, at) = self.repeat?;
        if now < at || self.rate == 0 {
            return None;
        }
        let interval = Duration::from_secs(1) / self.rate;
        // Never burst to catch up after a stall.
        let next = (at + interval).max(now);
        self.repeat = Some((key, next));
        self.xkb.as_ref().map(|x| xkb::Key {
            repeat: true,
            ..x.key(key)
        })
    }
}

impl<S: Default> Window<S> {
    /// Seat, keyboard and pointer events. Ok(false): not an input object.
    pub(super) fn on_input(&mut self, ev: &WlEvent) -> Result<bool, Error> {
        let i = &self.input;
        if ev.obj == self.ids.seat && ev.obj != 0 {
            self.on_seat(ev)?;
        } else if ev.obj == i.keyboard && ev.obj != 0 {
            self.on_keyboard(ev)?;
        } else if ev.obj == i.pointer && ev.obj != 0 {
            self.on_pointer(ev)?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    fn on_seat(&mut self, ev: &WlEvent) -> Result<(), Error> {
        if ev.op != seat::EV_CAPABILITIES {
            return Ok(());
        }
        let caps = ev.args().u32()?;
        let (s, c) = (self.ids.seat, &mut self.conn);
        if caps & seat::CAP_KEYBOARD != 0 && self.input.keyboard == 0 {
            self.input.keyboard = c.new_id();
            c.send(s, seat::GET_KEYBOARD, &[Arg::U(self.input.keyboard)])?;
        }
        if caps & seat::CAP_POINTER != 0 && self.input.pointer == 0 {
            self.input.pointer = c.new_id();
            c.send(s, seat::GET_POINTER, &[Arg::U(self.input.pointer)])?;
            if self.ids.cursor_shape != 0 {
                self.input.cursor_dev = c.new_id();
                let args = [Arg::U(self.input.cursor_dev), Arg::U(self.input.pointer)];
                c.send(self.ids.cursor_shape, cursor_shape::GET_POINTER, &args)?;
            }
        }
        Ok(())
    }

    fn on_keyboard(&mut self, ev: &WlEvent) -> Result<(), Error> {
        let mut a = ev.args();
        let inp = &mut self.input;
        match ev.op {
            keyboard::EV_KEYMAP => {
                let (format, size) = (a.u32()?, a.u32()? as usize);
                let fd = self.conn.take_fd()?;
                if format == keyboard::KEYMAP_XKB_V1 {
                    let map = Map::read_only(std::os::fd::AsFd::as_fd(&fd), size)?;
                    inp.xkb = Keyboard::new(map.bytes());
                }
            }
            keyboard::EV_ENTER => {
                inp.serial = a.u32()?;
                self.events.push_back(Event::Focus(true));
            }
            keyboard::EV_LEAVE => {
                inp.repeat = None;
                self.events.push_back(Event::Focus(false));
            }
            keyboard::EV_KEY => {
                let (serial, _time, key, state) = (a.u32()?, a.u32()?, a.u32()?, a.u32()?);
                inp.serial = serial;
                self.on_key(key, state == 1);
            }
            keyboard::EV_MODIFIERS => {
                let _serial = a.u32()?;
                let (d, l, k, g) = (a.u32()?, a.u32()?, a.u32()?, a.u32()?);
                if let Some(x) = &mut inp.xkb {
                    x.set_mods(d, l, k, g);
                }
            }
            keyboard::EV_REPEAT_INFO => {
                inp.rate = a.i32()?.max(0) as u32;
                inp.delay = Duration::from_millis(a.i32()?.max(0) as u64);
            }
            _ => {}
        }
        Ok(())
    }

    fn on_key(&mut self, key: u32, pressed: bool) {
        let inp = &mut self.input;
        let Some(x) = &inp.xkb else { return };
        if !pressed {
            if inp.repeat.is_some_and(|(k, _)| k == key) {
                inp.repeat = None;
            }
            return;
        }
        if x.repeats(key) && inp.rate > 0 {
            inp.repeat = Some((key, Instant::now() + inp.delay));
        }
        self.events.push_back(Event::Key(x.key(key)));
    }

    /// Show cursor `shape` (`proto::cursor_shape::SHAPE_*`) while over our window.
    pub fn set_cursor(&mut self, shape: u32) -> Result<(), Error> {
        let i = &mut self.input;
        if i.cursor_dev == 0 || i.shape == shape {
            return Ok(());
        }
        i.shape = shape;
        let args = [Arg::U(i.enter_serial), Arg::U(shape)];
        self.conn.send(i.cursor_dev, cursor_shape::SET_SHAPE, &args)
    }

    fn on_pointer(&mut self, ev: &WlEvent) -> Result<(), Error> {
        let mut a = ev.args();
        let inp = &mut self.input;
        let out = match ev.op {
            pointer::EV_ENTER => {
                // The shape is set by the app on the Motion below (`set_cursor`).
                inp.enter_serial = a.u32()?;
                inp.shape = 0;
                let _surface = a.u32()?;
                Some(Pointer::Motion(a.fixed()?, a.fixed()?))
            }
            pointer::EV_MOTION => {
                let _time = a.u32()?;
                Some(Pointer::Motion(a.fixed()?, a.fixed()?))
            }
            pointer::EV_BUTTON => {
                inp.serial = a.u32()?;
                let (_time, button, state) = (a.u32()?, a.u32()?, a.u32()?);
                Some(Pointer::Button {
                    button,
                    pressed: state == 1,
                })
            }
            pointer::EV_AXIS => {
                let (_time, axis, v) = (a.u32()?, a.u32()?, a.fixed()?);
                if axis == pointer::AXIS_VERTICAL {
                    inp.scroll_px += v;
                }
                None
            }
            pointer::EV_AXIS_DISCRETE => {
                let (axis, n) = (a.u32()?, a.i32()?);
                if axis == pointer::AXIS_VERTICAL {
                    inp.scroll_discrete += n;
                }
                None
            }
            pointer::EV_FRAME if inp.scroll_discrete != 0 || inp.scroll_px != 0.0 => {
                let s = Pointer::Scroll {
                    discrete: inp.scroll_discrete,
                    px: inp.scroll_px,
                };
                (inp.scroll_discrete, inp.scroll_px) = (0, 0.0);
                Some(s)
            }
            _ => None,
        };
        if let Some(p) = out {
            self.events.push_back(Event::Pointer(p));
        }
        Ok(())
    }
}
