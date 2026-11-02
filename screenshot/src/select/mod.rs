//! Interactive pick on a frozen screen: drag a region, or click the hovered window,
//! or click empty desktop for that whole output. Esc or right click cancels.

pub mod hypr;
mod overlay;

use crate::capture::{Error, Session, Shot};
use crate::wl::{self, Arg, Conn, Event, proto};
use hypr::Win;
use overlay::{Kit, Overlay};

/// What the user picked, in layout coordinates.
pub enum Pick {
    /// `[x, y, w, h]`
    Region([i32; 4]),
    /// A window's Hyprland handle (low 32 bits of its address).
    Window(u32),
}

/// A press that moves less than this is a click, not a drag.
const DRAG_PX: i32 = 4;

struct Ids {
    pointer: u32,
    keyboard: u32,
    cursor: Option<u32>,
}

struct State {
    ovs: Vec<Overlay>,
    wins: Vec<Win>,
    /// The overlay the pointer is on, and the pointer in layout coordinates.
    on: usize,
    pos: (i32, i32),
    drag: Option<(i32, i32)>,
    done: Option<Option<Pick>>,
}

/// Show the frozen `shots`, let the user pick. None = cancelled. The overlays are gone
/// (screen live again) when this returns.
pub fn run(s: &mut Session, shots: &[Shot], wins: Vec<Win>) -> Result<Option<Pick>, Error> {
    let kit = kit(s)?;
    let ids = input(s)?;
    let mut st = State {
        ovs: Vec::new(),
        wins,
        on: 0,
        pos: (0, 0),
        drag: None,
        done: None,
    };
    for shot in shots {
        st.ovs.push(Overlay::new(&mut s.c, &kit, shot)?);
    }
    while st.done.is_none() {
        s.c.pump(|c, ev| st.event(c, &ids, ev))?;
    }
    for o in &st.ovs {
        o.destroy(&mut s.c)?;
    }
    release(&mut s.c, &ids)?;
    // Roundtrip: the screen is live again before we encode, and any keymap event
    // still in flight gives up its fd here (a queued fd would misroute a paste).
    let cb = s.c.sync()?;
    let mut synced = false;
    while !synced {
        s.c.pump(|c, ev| {
            synced |= ev.obj == cb;
            st.event(c, &ids, ev)
        })?;
    }
    Ok(st.done.flatten())
}

/// Give back pointer, keyboard and cursor device (wl_seat v5 has `release`).
fn release(c: &mut Conn, ids: &Ids) -> Result<(), wl::Error> {
    if let Some(dev) = ids.cursor {
        c.send(dev, proto::cursor_shape::DESTROY_DEVICE, &[])?;
    }
    c.send(ids.pointer, proto::pointer::RELEASE, &[])?;
    c.send(ids.keyboard, proto::keyboard::RELEASE, &[])
}

fn need(s: &mut Session, name: &'static str, max: u32) -> Result<u32, Error> {
    Ok(s.bind(name, max)?.ok_or(wl::Error::Missing(name))?)
}

fn kit(s: &mut Session) -> Result<Kit, Error> {
    let compositor = need(s, proto::compositor::NAME, 4)?;
    let subcompositor = need(s, proto::subcompositor::NAME, 1)?;
    let layer_shell = need(s, proto::layer_shell::NAME, 3)?;
    let viewporter = need(s, proto::viewporter::NAME, 1)?;
    let sp = need(s, proto::single_pixel::NAME, 1)?;
    let mut pixel = |rgba: [u32; 4]| -> Result<u32, wl::Error> {
        let id = s.c.new_id();
        let [r, g, b, a] = rgba.map(Arg::U);
        s.c.send(
            sp,
            proto::single_pixel::CREATE_U32_RGBA_BUFFER,
            &[Arg::U(id), r, g, b, a],
        )?;
        Ok(id)
    };
    let (dim, line) = (pixel(overlay::DIM_RGBA)?, pixel(overlay::LINE_RGBA)?);
    let no_input = s.c.new_id();
    s.c.send(
        compositor,
        proto::compositor::CREATE_REGION,
        &[Arg::U(no_input)],
    )?;
    Ok(Kit {
        compositor,
        subcompositor,
        layer_shell,
        viewporter,
        dim,
        line,
        no_input,
    })
}

fn input(s: &mut Session) -> Result<Ids, Error> {
    let seat = need(s, proto::seat::NAME, 5)?;
    let (pointer, keyboard) = (s.c.new_id(), s.c.new_id());
    s.c.send(seat, proto::seat::GET_POINTER, &[Arg::U(pointer)])?;
    s.c.send(seat, proto::seat::GET_KEYBOARD, &[Arg::U(keyboard)])?;
    let cursor = match s.bind(proto::cursor_shape::NAME, 1)? {
        Some(m) => {
            let dev = s.c.new_id();
            s.c.send(
                m,
                proto::cursor_shape::GET_POINTER,
                &[Arg::U(dev), Arg::U(pointer)],
            )?;
            Some(dev)
        }
        None => None,
    };
    Ok(Ids {
        pointer,
        keyboard,
        cursor,
    })
}

impl State {
    fn event(&mut self, c: &mut Conn, ids: &Ids, ev: &Event) -> Result<(), wl::Error> {
        let mut a = ev.args();
        if ev.obj == ids.keyboard && ev.op == proto::keyboard::EV_KEYMAP {
            return c.take_fd().map(drop); // every keymap carries an fd; we only need Esc
        }
        if self.done.is_some() {
            return Ok(()); // picked or cancelled: later input must not change that
        }
        if ev.obj == ids.pointer {
            return self.pointer(c, ids, ev);
        }
        if ev.obj == ids.keyboard {
            if ev.op == proto::keyboard::EV_KEY {
                let (_, _, key, pressed) = (a.u32()?, a.u32()?, a.u32()?, a.u32()? == 1);
                if pressed && key == proto::keyboard::KEY_ESC {
                    self.done = Some(None);
                }
            }
            return Ok(());
        }
        if ev.op == proto::callback::EV_DONE {
            for o in &mut self.ovs {
                if o.frame_done(c, ev.obj)? {
                    return Ok(());
                }
            }
        }
        let Some(o) = self.ovs.iter_mut().find(|o| o.layer == ev.obj) else {
            return Ok(());
        };
        match ev.op {
            proto::layer_surface::EV_CONFIGURE => o.configure(c, a.u32()?),
            proto::layer_surface::EV_CLOSED => {
                self.done = Some(None);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn pointer(&mut self, c: &mut Conn, ids: &Ids, ev: &Event) -> Result<(), wl::Error> {
        let mut a = ev.args();
        match ev.op {
            proto::pointer::EV_ENTER => {
                let (serial, surf, fx, fy) = (a.u32()?, a.u32()?, a.i32()?, a.i32()?);
                if let Some(dev) = ids.cursor {
                    let shape = [Arg::U(serial), Arg::U(proto::cursor_shape::CROSSHAIR)];
                    c.send(dev, proto::cursor_shape::SET_SHAPE, &shape)?;
                }
                self.on = self.ovs.iter().position(|o| o.surf == surf).unwrap_or(0);
                self.moved(c, fx, fy)
            }
            proto::pointer::EV_MOTION => {
                let (_, fx, fy) = (a.u32()?, a.i32()?, a.i32()?);
                self.moved(c, fx, fy)
            }
            proto::pointer::EV_BUTTON => {
                let (_, _, button, pressed) = (a.u32()?, a.u32()?, a.u32()?, a.u32()? == 1);
                self.button(c, button, pressed)
            }
            _ => Ok(()),
        }
    }

    /// Pointer moved to surface-local wl_fixed (24.8) `fx`, `fy` on overlay `on`.
    fn moved(&mut self, c: &mut Conn, fx: i32, fy: i32) -> Result<(), wl::Error> {
        let [ox, oy, _, _] = self.ovs.get(self.on).map_or([0; 4], |o| o.rect);
        self.pos = (ox + (fx >> 8), oy + (fy >> 8));
        self.redraw(c)
    }

    fn button(&mut self, c: &mut Conn, button: u32, pressed: bool) -> Result<(), wl::Error> {
        match (button, pressed) {
            (proto::pointer::BTN_RIGHT, true) => self.done = Some(None),
            (proto::pointer::BTN_LEFT, true) => self.drag = Some(self.pos),
            (proto::pointer::BTN_LEFT, false) => {
                self.done = Some(Some(self.pick()));
                self.drag = None;
            }
            _ => {}
        }
        self.redraw(c)
    }

    /// On release: a drag is a region; a click is the window under it, else its output.
    fn pick(&self) -> Pick {
        if let Some(r) = self.dragged() {
            return Pick::Region(r);
        }
        match hypr::hit(&self.wins, self.pos.0, self.pos.1) {
            Some(w) => Pick::Window(w.addr as u32),
            None => Pick::Region(self.ovs.get(self.on).map_or([0, 0, 1, 1], |o| o.rect)),
        }
    }

    /// The dragged rectangle, once the press moved at least DRAG_PX.
    fn dragged(&self) -> Option<[i32; 4]> {
        let ((x0, y0), (x1, y1)) = (self.drag?, self.pos);
        let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
        (w >= DRAG_PX || h >= DRAG_PX).then(|| [x0.min(x1), y0.min(y1), w.max(1), h.max(1)])
    }

    /// The hole is the drag, else the hovered window, else nothing.
    fn redraw(&mut self, c: &mut Conn) -> Result<(), wl::Error> {
        let hover = || hypr::hit(&self.wins, self.pos.0, self.pos.1).map(|w| w.rect);
        let hole = self.dragged().or_else(hover);
        for o in &mut self.ovs {
            o.set_hole(c, hole)?;
        }
        Ok(())
    }
}
