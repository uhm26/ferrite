//! Grab outputs (wlr-screencopy) or one window (Hyprland toplevel export) into shared
//! memory. A `Session` is one connection: two roundtrips to learn globals and outputs,
//! then all frames are requested at once so outputs copy in parallel.

mod frame;
mod shm;

use std::fmt;
use std::io;

use crate::wl::{self, Arg, Conn, Event, Global, proto};
use frame::Frame;

#[derive(Debug)]
pub enum Error {
    Wl(wl::Error),
    Io(io::Error),
    NoOutput(String),
    Failed(String),
    /// The compositor offered no 8-bit RGB shm format; the last one it offered.
    Format(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Wl(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "shared memory: {e}"),
            Error::NoOutput(n) if n.is_empty() => write!(f, "compositor has no outputs"),
            Error::NoOutput(n) => write!(f, "no output named {n}"),
            Error::Failed(n) => write!(f, "compositor failed to copy {n}"),
            Error::Format(x) => write!(f, "unsupported pixel format {x:#x}"),
        }
    }
}

impl From<wl::Error> for Error {
    fn from(e: wl::Error) -> Self {
        Error::Wl(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// One captured output or window: where it sits in the layout, and its pixels.
pub struct Shot {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    /// Bytes are B,G,R,X in memory (wl_shm (A|X)RGB8888); else R,G,B,X.
    pub bgr: bool,
    pub y_invert: bool,
    /// The wl_buffer holding the pixels, ready to attach to a surface.
    pub buf: u32,
    /// The wl_output this came from (0 for a window).
    pub output: u32,
    map: shm::Map,
}

impl Shot {
    pub fn bytes(&self) -> &[u8] {
        self.map.bytes()
    }

    fn from_frame(f: Frame, output: u32, x: i32, y: i32) -> Option<Self> {
        let (format, w, h, stride) = f.layout?;
        let (y_invert, buf, map) = (f.y_invert, f.buf, f.map?);
        let bgr = format <= 1;
        Some(Shot {
            x,
            y,
            w,
            h,
            stride,
            bgr,
            y_invert,
            buf,
            output,
            map,
        })
    }
}

struct Output {
    id: u32,
    name: String,
    x: i32,
    y: i32,
}

/// One connection with the globals screenshot uses.
pub struct Session {
    pub c: Conn,
    reg: u32,
    globals: Vec<Global>,
    shm: u32,
    outs: Vec<Output>,
}

impl Session {
    pub fn connect() -> Result<Self, Error> {
        let mut c = Conn::connect()?;
        let reg = c.get_registry()?;
        let mut globals = Vec::new();
        roundtrip(&mut c, |ev| {
            if ev.obj == reg && ev.op == proto::registry::EV_GLOBAL {
                let mut a = ev.args();
                let (name, interface, version) = (a.u32()?, a.str()?.to_owned(), a.u32()?);
                globals.push(Global {
                    name,
                    interface,
                    version,
                });
            }
            Ok(())
        })?;
        let mut s = Session {
            c,
            reg,
            globals,
            shm: 0,
            outs: Vec::new(),
        };
        s.shm = s
            .bind(proto::shm::NAME, 1)?
            .ok_or(wl::Error::Missing("wl_shm"))?;
        s.outs = s.outputs()?;
        Ok(s)
    }

    /// Bind the global `name` at version ≤ `max`, if the compositor has it.
    pub fn bind(&mut self, name: &str, max: u32) -> Result<Option<u32>, Error> {
        let Some(g) = self.globals.iter().find(|g| g.interface == name) else {
            return Ok(None);
        };
        let g = g.clone();
        Ok(Some(self.c.bind(self.reg, &g, max)?))
    }

    /// Capture every output, or only the one named `want`.
    pub fn grab_outputs(&mut self, want: Option<&str>) -> Result<Vec<Shot>, Error> {
        let sc = match self
            .globals
            .iter()
            .find(|g| g.interface == proto::screencopy::NAME)
        {
            Some(g) if g.version >= 3 => self.bind(proto::screencopy::NAME, 3)?,
            _ => None,
        };
        let sc = sc.ok_or(wl::Error::Missing("zwlr_screencopy_manager_v1 v3"))?;
        let outs: Vec<&Output> = self
            .outs
            .iter()
            .filter(|o| want.is_none_or(|w| o.name == w))
            .collect();
        if outs.is_empty() {
            return Err(Error::NoOutput(want.unwrap_or_default().to_owned()));
        }
        let mut frames = Vec::new();
        for o in &outs {
            let id = self.c.new_id();
            let args = [Arg::U(id), Arg::I(0), Arg::U(o.id)];
            self.c.send(sc, proto::screencopy::CAPTURE_OUTPUT, &args)?;
            frames.push(Frame::new(id));
        }
        let labels: Vec<String> = outs.iter().map(|o| format!("output {}", o.name)).collect();
        let place: Vec<(u32, i32, i32)> = outs.iter().map(|o| (o.id, o.x, o.y)).collect();
        frame::wait_all(
            &mut self.c,
            self.shm,
            &frame::SCREENCOPY,
            &mut frames,
            &labels,
        )?;
        let shots = frames.into_iter().zip(place).zip(labels);
        let shot = |((f, (id, x, y)), l)| Shot::from_frame(f, id, x, y).ok_or(Error::Failed(l));
        shots.map(shot).collect() // a missing output is an error, not a smaller image
    }

    /// Capture one window alone, even where others cover it. `handle` is the low 32
    /// bits of its Hyprland address (as `hyprctl clients` shows it).
    pub fn grab_window(&mut self, handle: u32) -> Result<Shot, Error> {
        let te = self.bind(proto::toplevel_export::NAME, 1)?;
        let te = te.ok_or(wl::Error::Missing("hyprland_toplevel_export_manager_v1"))?;
        let id = self.c.new_id();
        let args = [Arg::U(id), Arg::I(0), Arg::U(handle)];
        self.c
            .send(te, proto::toplevel_export::CAPTURE_TOPLEVEL, &args)?;
        let mut frames = [Frame::new(id)];
        let labels = ["the window".to_owned()];
        frame::wait_all(
            &mut self.c,
            self.shm,
            &frame::TOPLEVEL,
            &mut frames,
            &labels,
        )?;
        let [f] = frames;
        Shot::from_frame(f, 0, 0, 0).ok_or(Error::Failed(labels[0].clone()))
    }

    /// Bind every wl_output and learn its name and position.
    fn outputs(&mut self) -> Result<Vec<Output>, Error> {
        let mut outs = Vec::new();
        let gs: Vec<Global> = self
            .globals
            .iter()
            .filter(|g| g.interface == proto::output::NAME)
            .cloned()
            .collect();
        for g in &gs {
            let id = self.c.bind(self.reg, g, 4)?;
            outs.push(Output {
                id,
                name: String::new(),
                x: 0,
                y: 0,
            });
        }
        roundtrip(&mut self.c, |ev| {
            let Some(o) = outs.iter_mut().find(|o| o.id == ev.obj) else {
                return Ok(());
            };
            let mut a = ev.args();
            match ev.op {
                proto::output::EV_GEOMETRY => (o.x, o.y) = (a.i32()?, a.i32()?),
                proto::output::EV_NAME => a.str()?.clone_into(&mut o.name),
                _ => {}
            }
            Ok(())
        })?;
        Ok(outs)
    }
}

/// Send a sync and hand every event before its `done` to `f`.
fn roundtrip(
    c: &mut Conn,
    mut f: impl FnMut(&Event) -> Result<(), wl::Error>,
) -> Result<(), wl::Error> {
    let cb = c.sync()?;
    let mut done = false;
    while !done {
        c.pump(|_, ev| {
            if ev.obj == cb && ev.op == proto::callback::EV_DONE {
                done = true;
                return Ok(());
            }
            f(ev)
        })?;
    }
    Ok(())
}
