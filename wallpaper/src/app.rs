//! The daemon: one background-layer surface per output, all showing one image.
//! Outputs with the same pixel size share one buffer, so two 1080p screens cost
//! one decode, one scale and one 8 MB buffer. Our copy is unmapped right after
//! the compositor has it, and the buffer is destroyed when the compositor lets go.

use std::collections::HashMap;
use std::fmt;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::image::{self, Rgb};
use crate::wl::proto::{buffer, display, layer_surface, output, registry};
use crate::wl::{self, Conn, Event, Global};
use crate::{ipc, scale, shm, sys};

#[derive(Debug)]
pub enum Error {
    Wayland(wl::Error),
    Image(image::Error),
    Io(std::io::Error),
    Missing(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Wayland(e) => e.fmt(f),
            Error::Image(e) => e.fmt(f),
            Error::Io(e) => write!(f, "{e}"),
            Error::Missing(what) => write!(f, "compositor has no {what}"),
        }
    }
}

impl From<wl::Error> for Error {
    fn from(e: wl::Error) -> Self {
        Error::Wayland(e)
    }
}
impl From<image::Error> for Error {
    fn from(e: image::Error) -> Self {
        Error::Image(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// What a client-side object id is, for routing events. `u32` = output's global name.
enum Obj {
    Registry,
    Callback,
    Output(u32),
    Layer(u32),
    Buffer,
}

struct Screen {
    name: u32,
    output: u32,
    version: u32,
    scale: i32,
    ready: bool,
    closed: bool,
    surf: Option<Surf>,
}

struct Surf {
    wl: u32,
    layer: u32,
    /// Logical size from the last configure; 0 until then.
    size: (u32, u32),
    dirty: bool,
}

struct App {
    conn: Conn,
    registry: u32,
    compositor: u32,
    shm: u32,
    shell: u32,
    screens: Vec<Screen>,
    objs: HashMap<u32, Obj>,
    image: PathBuf,
    synced: Option<u32>,
}

/// Show `image` on every output and serve wallpaper changes until the compositor quits.
pub fn run(image: PathBuf, listener: &UnixListener) -> Result<(), Error> {
    let t = Instant::now();
    let first = image::load(&image)?;
    let decode = t.elapsed();
    let mut app = App::connect(image)?;
    app.draw(Some(first), decode)?;
    ipc::remember(&app.image);
    let r = app.serve(listener);
    ipc::remove_socket();
    match r {
        Err(Error::Wayland(wl::Error::Closed)) => Ok(()),
        other => other,
    }
}

/// Only a broken compositor connection ends the daemon; anything else is logged.
fn survivable(r: Result<(), Error>) -> Result<(), Error> {
    match r {
        Err(Error::Wayland(e)) => Err(Error::Wayland(e)),
        Err(e) => {
            eprintln!("wall: {e}");
            Ok(())
        }
        Ok(()) => Ok(()),
    }
}

impl App {
    fn connect(image: PathBuf) -> Result<Self, Error> {
        let mut conn = Conn::connect()?;
        let registry = conn.get_registry()?;
        let mut app = App {
            conn,
            registry,
            compositor: 0,
            shm: 0,
            shell: 0,
            screens: Vec::new(),
            objs: HashMap::from([(registry, Obj::Registry)]),
            image,
            synced: None,
        };
        app.roundtrip()?; // globals → binds (outputs start sending their info)
        for (id, what) in [(app.compositor, "wl_compositor v4"), (app.shm, "wl_shm")] {
            if id == 0 {
                return Err(Error::Missing(what));
            }
        }
        if app.shell == 0 {
            return Err(Error::Missing("wlr layer shell"));
        }
        app.roundtrip()?; // output info → surfaces
        app.ensure_surfaces()?;
        app.roundtrip()?; // first configures
        Ok(app)
    }

    fn roundtrip(&mut self) -> Result<(), Error> {
        let cb = self.conn.sync()?;
        self.objs.insert(cb, Obj::Callback);
        self.conn.flush()?;
        while self.synced != Some(cb) {
            self.conn.read()?;
            self.dispatch_all()?;
        }
        Ok(())
    }

    fn dispatch_all(&mut self) -> Result<(), Error> {
        while let Some(ev) = self.conn.next_event()? {
            self.dispatch(&ev)?;
        }
        Ok(())
    }

    fn dispatch(&mut self, ev: &Event) -> Result<(), Error> {
        if ev.obj == wl::proto::DISPLAY {
            return self.on_display(ev);
        }
        match self.objs.get(&ev.obj) {
            Some(Obj::Registry) => self.on_registry(ev),
            Some(Obj::Callback) => {
                self.objs.remove(&ev.obj);
                self.synced = Some(ev.obj);
                Ok(())
            }
            Some(&Obj::Output(name)) => self.on_output(name, ev),
            Some(&Obj::Layer(name)) => self.on_layer(name, ev),
            Some(Obj::Buffer) if ev.op == buffer::EV_RELEASE => {
                self.objs.remove(&ev.obj);
                Ok(self.conn.destroy_buffer(ev.obj)?)
            }
            _ => Ok(()), // surface enter/leave, shm formats, …: not needed
        }
    }

    fn on_display(&mut self, ev: &Event) -> Result<(), Error> {
        let mut a = ev.args();
        match ev.op {
            display::EV_ERROR => {
                let (object, code) = (a.u32()?, a.u32()?);
                let message = a.str()?.to_string();
                Err(wl::Error::Protocol {
                    object,
                    code,
                    message,
                }
                .into())
            }
            display::EV_DELETE_ID => {
                self.objs.remove(&a.u32()?);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn on_registry(&mut self, ev: &Event) -> Result<(), Error> {
        let mut a = ev.args();
        if ev.op == registry::EV_GLOBAL_REMOVE {
            let name = a.u32()?;
            return self.remove_screen(name);
        }
        if ev.op != registry::EV_GLOBAL {
            return Ok(());
        }
        let g = Global {
            name: a.u32()?,
            interface: a.str()?.to_string(),
            version: a.u32()?,
        };
        let reg = self.registry;
        match g.interface.as_str() {
            "wl_compositor" if g.version >= 4 => self.compositor = self.conn.bind(reg, &g, 4)?,
            "wl_shm" => self.shm = self.conn.bind(reg, &g, 1)?,
            "zwlr_layer_shell_v1" => self.shell = self.conn.bind(reg, &g, 1)?,
            "wl_output" if g.version >= 2 => {
                let id = self.conn.bind(reg, &g, 4)?;
                self.objs.insert(id, Obj::Output(g.name));
                let version = g.version.min(4);
                self.screens.push(Screen {
                    name: g.name,
                    output: id,
                    version,
                    scale: 1,
                    ready: false,
                    closed: false,
                    surf: None,
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn screen(&mut self, name: u32) -> Option<&mut Screen> {
        self.screens.iter_mut().find(|s| s.name == name)
    }

    fn on_output(&mut self, name: u32, ev: &Event) -> Result<(), Error> {
        let Some(s) = self.screen(name) else {
            return Ok(());
        };
        match ev.op {
            output::EV_SCALE => {
                let scale = ev.args().i32()?.max(1);
                if scale != s.scale {
                    s.scale = scale;
                    if let Some(surf) = &mut s.surf {
                        surf.dirty = surf.size != (0, 0);
                    }
                }
            }
            output::EV_DONE => s.ready = true,
            _ => {}
        }
        Ok(())
    }

    fn on_layer(&mut self, name: u32, ev: &Event) -> Result<(), Error> {
        let Some(i) = self.screens.iter().position(|s| s.name == name) else {
            return Ok(());
        };
        let mut a = ev.args();
        match ev.op {
            layer_surface::EV_CONFIGURE => {
                let (serial, w, h) = (a.u32()?, a.u32()?, a.u32()?);
                let Some(surf) = self.screens[i].surf.as_mut() else {
                    return Ok(());
                };
                let layer = surf.layer;
                if (w, h) != surf.size {
                    surf.size = (w, h);
                    surf.dirty = true;
                }
                self.conn.ack_configure(layer, serial)?;
            }
            layer_surface::EV_CLOSED => {
                if let Some(surf) = self.screens[i].surf.take() {
                    self.objs.remove(&surf.layer);
                    self.conn.destroy_layer(surf.layer, surf.wl)?;
                }
                self.screens[i].closed = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn ensure_surfaces(&mut self) -> Result<(), Error> {
        for i in 0..self.screens.len() {
            let s = &self.screens[i];
            if !s.ready || s.closed || s.surf.is_some() {
                continue;
            }
            let (name, output) = (s.name, s.output);
            let wl = self.conn.create_surface(self.compositor)?;
            let layer = self.conn.background_layer(self.shell, wl, output)?;
            self.objs.insert(layer, Obj::Layer(name));
            self.screens[i].surf = Some(Surf {
                wl,
                layer,
                size: (0, 0),
                dirty: false,
            });
        }
        Ok(())
    }

    fn remove_screen(&mut self, name: u32) -> Result<(), Error> {
        let Some(i) = self.screens.iter().position(|s| s.name == name) else {
            return Ok(());
        };
        let s = self.screens.remove(i);
        if let Some(surf) = s.surf {
            self.objs.remove(&surf.layer);
            self.conn.destroy_layer(surf.layer, surf.wl)?;
        }
        self.objs.remove(&s.output);
        if s.version >= 3 {
            self.conn.release_output(s.output)?;
        }
        Ok(())
    }

    /// Wait for compositor events and wallpaper requests, forever.
    fn serve(&mut self, listener: &UnixListener) -> Result<(), Error> {
        loop {
            self.conn.flush()?;
            let mut fds = [
                sys::PollFd {
                    fd: self.conn.fd(),
                    events: sys::POLLIN,
                    revents: 0,
                },
                sys::PollFd {
                    fd: listener.as_raw_fd(),
                    events: sys::POLLIN,
                    revents: 0,
                },
            ];
            sys::wait(&mut fds)?;
            if fds[0].revents != 0 {
                self.conn.read()?;
                self.dispatch_all()?;
                self.ensure_surfaces()?;
                // A failed redraw (image moved away, out of memory) leaves the
                // surfaces dirty, so the next compositor event retries it.
                survivable(self.draw(None, Default::default()))?;
            }
            if fds[1].revents != 0 {
                self.serve_client(listener)?;
            }
        }
    }

    fn serve_client(&mut self, listener: &UnixListener) -> Result<(), Error> {
        let mut asks = Vec::new();
        while let Some(ask) = ipc::accept(listener)? {
            asks.push(ask);
        }
        // Newest first: once one is shown, the ones queued before it are moot.
        while let Some(ask) = asks.pop() {
            match self.switch(&ask.image) {
                Ok(()) => {
                    ask.answer(Ok(()));
                    for old in asks.drain(..) {
                        old.answer(Err("skipped: a newer wallpaper came in".into()));
                    }
                }
                Err(e) => {
                    ask.answer(Err(e.to_string()));
                    if matches!(e, Error::Wayland(_)) {
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    }

    /// Decode `image` and draw it on every output.
    fn switch(&mut self, image: &Path) -> Result<(), Error> {
        let t = Instant::now();
        let img = image::load(image)?;
        self.image = image.to_path_buf();
        for surf in self.screens.iter_mut().filter_map(|s| s.surf.as_mut()) {
            surf.dirty = surf.size != (0, 0);
        }
        self.draw(Some(img), t.elapsed())?;
        ipc::remember(&self.image);
        Ok(())
    }

    /// Draw every dirty surface. `img`: already decoded, else read from disk.
    fn draw(&mut self, img: Option<Rgb>, decode: std::time::Duration) -> Result<(), Error> {
        let mut groups: Vec<((usize, usize), Vec<usize>)> = Vec::new();
        for (i, s) in self.screens.iter().enumerate() {
            let Some(surf) = s.surf.as_ref().filter(|f| f.dirty && f.size != (0, 0)) else {
                continue;
            };
            let px = (
                surf.size.0 as usize * s.scale as usize,
                surf.size.1 as usize * s.scale as usize,
            );
            match groups.iter_mut().find(|g| g.0 == px) {
                Some(g) => g.1.push(i),
                None => groups.push((px, vec![i])),
            }
        }
        if groups.is_empty() {
            return Ok(());
        }
        let (img, decode) = match img {
            Some(img) => (img, decode),
            None => {
                let t = Instant::now();
                (image::load(&self.image)?, t.elapsed())
            }
        };
        let t = Instant::now();
        for ((w, h), screens) in &groups {
            let mut px = shm::Pixels::new(w * h * 4)?;
            scale::cover(&img, *w, *h, px.as_u32_mut());
            let buf = self
                .conn
                .shm_buffer(self.shm, px.fd(), *w as i32, *h as i32)?;
            self.objs.insert(buf, Obj::Buffer);
            for &i in screens {
                let s = &mut self.screens[i];
                let Some(surf) = s.surf.as_mut() else {
                    continue;
                };
                surf.dirty = false;
                self.conn.present(self.compositor, surf.wl, buf, s.scale)?;
            }
        }
        self.conn.flush()?;
        let n: usize = groups.iter().map(|g| g.1.len()).sum();
        eprintln!(
            "wall: {} on {n} output(s): {}x{} decode {:.1} ms, scale+upload {:.1} ms ({} buffer(s))",
            self.image.display(),
            img.w,
            img.h,
            decode.as_secs_f64() * 1e3,
            t.elapsed().as_secs_f64() * 1e3,
            groups.len()
        );
        drop(img);
        sys::trim_heap(); // the decoded image was MBs; hand them back to the OS
        Ok(())
    }
}
