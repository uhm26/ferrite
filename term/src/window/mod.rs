//! Our Wayland window: one xdg toplevel showing shm buffers. Turns protocol events
//! into a few [`Event`]s for the app. Input lives in `input.rs`, copy/paste in
//! `clipboard.rs`, the buffer pool in `buffers.rs`.

mod buffers;
mod clipboard;
mod input;

pub use buffers::Buffer;
pub use input::Pointer;

use std::collections::VecDeque;
use std::time::Instant;

use crate::render::draw::Rect;
use crate::sys;
use crate::wl::proto::{self, callback, display, registry, surface, toplevel, wm_base, xdg_surface};
use crate::wl::{Arg, Conn, Error, Event as WlEvent, Global};
use crate::xkb::Key;

pub enum Event {
    /// The compositor chose a size (already acknowledged).
    Resize(usize, usize),
    Close,
    /// The last frame was shown; drawing again is welcome.
    Frame,
    /// A buffer came back; a pending draw may proceed.
    Released,
    Key(Key),
    Focus(bool),
    Pointer(Pointer),
}

/// Object ids of the globals we bound.
#[derive(Default)]
struct Ids {
    registry: u32,
    compositor: u32,
    shm: u32,
    wm_base: u32,
    seat: u32,
    ddm: u32,
    cursor_shape: u32,
}

pub struct Window {
    conn: Conn,
    ids: Ids,
    surface: u32,
    xdg: u32,
    toplevel: u32,
    pub width: usize,
    pub height: usize,
    pending_size: (usize, usize),
    frame_cb: Option<u32>,
    buffers: Vec<Buffer>,
    pool: Option<buffers::Pool>,
    input: input::Input,
    clip: clipboard::Clipboard,
    events: VecDeque<Event>,
    globals: Vec<Global>,
    sync_cb: u32,
}

impl Window {
    /// `opaque`: the window never shows through (lets the compositor skip what's
    /// behind it). False for a glass background.
    pub fn new(title: &str, app_id: &str, opaque: bool) -> Result<Self, Error> {
        let mut conn = Conn::connect()?;
        sys::set_nonblocking(conn.fd())?;
        let registry = conn.get_registry()?;
        let mut w = Window {
            conn,
            ids: Ids {
                registry,
                ..Ids::default()
            },
            surface: 0,
            xdg: 0,
            toplevel: 0,
            width: 0,
            height: 0,
            pending_size: (0, 0),
            frame_cb: None,
            buffers: Vec::new(),
            pool: None,
            input: input::Input::default(),
            clip: clipboard::Clipboard::default(),
            events: VecDeque::new(),
            globals: Vec::new(),
            sync_cb: 0,
        };
        w.roundtrip()?;
        w.bind_globals()?;
        w.create_surface(title, app_id, opaque)?;
        w.roundtrip()?; // seat capabilities, data device, first configure
        Ok(w)
    }

    fn bind_globals(&mut self) -> Result<(), Error> {
        let globals = std::mem::take(&mut self.globals);
        let reg = self.ids.registry;
        for g in &globals {
            let max = match g.interface.as_str() {
                "wl_compositor" => 4,
                "wl_shm" => 1,
                "xdg_wm_base" => 2,
                "wl_seat" => 7, // 8 replaces axis_discrete with axis_value120
                "wl_data_device_manager" => 3,
                "wp_cursor_shape_manager_v1" => 1,
                _ => continue,
            };
            let id = self.conn.bind(reg, g, max)?;
            let slot = match g.interface.as_str() {
                "wl_compositor" => &mut self.ids.compositor,
                "wl_shm" => &mut self.ids.shm,
                "xdg_wm_base" => &mut self.ids.wm_base,
                "wl_seat" => &mut self.ids.seat,
                "wl_data_device_manager" => &mut self.ids.ddm,
                _ => &mut self.ids.cursor_shape,
            };
            if *slot == 0 {
                *slot = id;
            }
        }
        for (id, name) in [
            (self.ids.compositor, "wl_compositor v4"),
            (self.ids.shm, "wl_shm"),
            (self.ids.wm_base, "xdg_wm_base"),
        ] {
            if id == 0 {
                return Err(Error::Missing(name));
            }
        }
        if self.ids.seat != 0 && self.ids.ddm != 0 {
            self.clip.device = self.conn.data_device(self.ids.ddm, self.ids.seat)?;
        }
        Ok(())
    }

    fn create_surface(&mut self, title: &str, app_id: &str, opaque: bool) -> Result<(), Error> {
        let c = &mut self.conn;
        self.surface = c.create_surface(self.ids.compositor)?;
        if opaque {
            c.opaque_everywhere(self.ids.compositor, self.surface)?;
        }
        self.xdg = c.new_id();
        let args = [Arg::U(self.xdg), Arg::U(self.surface)];
        c.send(self.ids.wm_base, wm_base::GET_XDG_SURFACE, &args)?;
        self.toplevel = c.new_id();
        c.send(self.xdg, xdg_surface::GET_TOPLEVEL, &[Arg::U(self.toplevel)])?;
        c.send(self.toplevel, toplevel::SET_TITLE, &[Arg::S(title)])?;
        c.send(self.toplevel, toplevel::SET_APP_ID, &[Arg::S(app_id)])?;
        c.send(self.surface, surface::COMMIT, &[])?;
        c.flush()
    }

    /// Send a sync and handle events until it comes back.
    fn roundtrip(&mut self) -> Result<(), Error> {
        self.sync_cb = self.conn.sync()?;
        self.conn.flush()?;
        while self.sync_cb != 0 {
            let mut fds = [sys::PollFd::new(self.conn.fd(), sys::POLLIN)];
            sys::wait(&mut fds, -1)?;
            self.dispatch()?;
        }
        Ok(())
    }

    pub fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.conn.fd()
    }

    /// Read and handle everything the compositor sent.
    pub fn dispatch(&mut self) -> Result<(), Error> {
        while self.conn.read()? {}
        while let Some(ev) = self.conn.next_event()? {
            self.handle(&ev)?;
        }
        Ok(())
    }

    /// Events already queued (the app must not sleep on the socket while true).
    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn next_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    pub fn flush(&mut self) -> Result<(), Error> {
        self.conn.flush()
    }

    fn handle(&mut self, ev: &WlEvent) -> Result<(), Error> {
        let mut a = ev.args();
        match (ev.obj, ev.op) {
            (proto::DISPLAY, display::EV_ERROR) => {
                let (object, code) = (a.u32()?, a.u32()?);
                let message = a.str()?.to_string();
                return Err(Error::Protocol { object, code, message });
            }
            (proto::DISPLAY, display::EV_DELETE_ID) => {
                let id = a.u32()?;
                self.clip.forget(id);
                self.conn.release_id(id);
            }
            (proto::DISPLAY, _) => {}
            (o, registry::EV_GLOBAL) if o == self.ids.registry => {
                let (name, interface, version) = (a.u32()?, a.str()?.to_string(), a.u32()?);
                self.globals.push(Global { name, interface, version });
            }
            (o, callback::EV_DONE) if o == self.sync_cb => self.sync_cb = 0,
            (o, callback::EV_DONE) if Some(o) == self.frame_cb => {
                self.frame_cb = None;
                self.events.push_back(Event::Frame);
            }
            (o, wm_base::EV_PING) if o == self.ids.wm_base => {
                let serial = a.u32()?;
                self.conn.send(o, wm_base::PONG, &[Arg::U(serial)])?;
            }
            (o, _) if o == self.xdg || o == self.toplevel => self.on_shell(ev)?,
            _ => self.handle_other(ev)?,
        }
        Ok(())
    }

    fn handle_other(&mut self, ev: &WlEvent) -> Result<(), Error> {
        if self.on_buffer(ev) {
            self.events.push_back(Event::Released);
            return Ok(());
        }
        if self.on_input(ev)? {
            return Ok(());
        }
        self.on_clipboard(ev)
    }

    fn on_shell(&mut self, ev: &WlEvent) -> Result<(), Error> {
        let mut a = ev.args();
        match (ev.obj == self.toplevel, ev.op) {
            (true, toplevel::EV_CONFIGURE) => {
                let (w, h) = (a.i32()?, a.i32()?);
                self.pending_size = (w.max(0) as usize, h.max(0) as usize);
            }
            (true, toplevel::EV_CLOSE) => self.events.push_back(Event::Close),
            (false, xdg_surface::EV_CONFIGURE) => {
                let serial = a.u32()?;
                self.conn.send(self.xdg, xdg_surface::ACK_CONFIGURE, &[Arg::U(serial)])?;
                let (w, h) = self.pending_size;
                self.events.push_back(Event::Resize(w, h));
            }
            _ => {}
        }
        Ok(())
    }

    /// Size from the first configure (0 = we choose), known once `new` returns.
    pub fn configured_size(&self) -> (usize, usize) {
        self.pending_size
    }

    /// Tell the compositor whether it may skip what is behind us (solid) or must
    /// blend it (glass). Takes effect with the next commit.
    pub fn set_opaque(&mut self, opaque: bool) {
        let r = if opaque {
            self.conn.opaque_everywhere(self.ids.compositor, self.surface)
        } else {
            self.conn.send(self.surface, surface::SET_OPAQUE_REGION, &[Arg::U(0)])
        };
        if let Err(e) = r {
            eprintln!("term: {e}");
        }
    }

    pub fn set_title(&mut self, title: &str) -> Result<(), Error> {
        self.conn.send(self.toplevel, toplevel::SET_TITLE, &[Arg::S(title)])
    }

    /// The size buffers are made at from now on.
    pub fn set_size(&mut self, w: usize, h: usize) {
        (self.width, self.height) = (w, h);
    }

    pub fn frame_pending(&self) -> bool {
        self.frame_cb.is_some()
    }

    /// Show buffer `i`: attach, damage, ask for a frame callback, commit.
    pub fn present(&mut self, i: usize, damage: &[Rect]) -> Result<(), Error> {
        let id = self.buffers[i].wl_id();
        self.buffers[i].busy = true;
        let c = &mut self.conn;
        c.send(self.surface, surface::ATTACH, &[Arg::U(id), Arg::I(0), Arg::I(0)])?;
        let bound;
        let rects = if damage.len() > 32 {
            bound = [bounding(damage)];
            &bound[..]
        } else {
            damage
        };
        for r in rects {
            let args = [r.x, r.y, r.w, r.h].map(|v| Arg::I(v as i32));
            c.send(self.surface, surface::DAMAGE_BUFFER, &args)?;
        }
        let cb = c.new_id();
        c.send(self.surface, surface::FRAME, &[Arg::U(cb)])?;
        self.frame_cb = Some(cb);
        c.send(self.surface, surface::COMMIT, &[])?;
        c.flush()
    }

    /// When the app should wake up for key repeat, if a key is held.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.input.repeat_deadline()
    }

    pub fn tick(&mut self, now: Instant) {
        if let Some(key) = self.input.repeat_due(now) {
            self.events.push_back(Event::Key(key));
        }
    }
}

fn bounding(rs: &[Rect]) -> Rect {
    let x0 = rs.iter().map(|r| r.x).min().unwrap_or(0);
    let y0 = rs.iter().map(|r| r.y).min().unwrap_or(0);
    let x1 = rs.iter().map(|r| r.x + r.w).max().unwrap_or(0);
    let y1 = rs.iter().map(|r| r.y + r.h).max().unwrap_or(0);
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}
