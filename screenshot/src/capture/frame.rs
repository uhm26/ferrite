//! One capture frame, the same dance for wlr-screencopy (outputs) and Hyprland's
//! toplevel export (windows): the compositor names a buffer layout, we answer with a
//! fresh shm buffer and `copy`, then wait for `ready`.

use super::Error;
use super::shm::Map;
use crate::wl::{Arg, Conn, Event, proto};

/// Event opcodes and copy arguments of one capture protocol.
pub struct Kind {
    buffer: u16,
    flags: u16,
    ready: u16,
    failed: u16,
    buffer_done: u16,
    /// toplevel export's `copy` takes an extra `ignore_damage` int.
    ignore_damage_arg: bool,
}

pub const SCREENCOPY: Kind = Kind {
    buffer: 0,
    flags: 1,
    ready: 2,
    failed: 3,
    buffer_done: 6,
    ignore_damage_arg: false,
};

pub const TOPLEVEL: Kind = Kind {
    buffer: 0,
    flags: 2,
    ready: 3,
    failed: 4,
    buffer_done: 6,
    ignore_damage_arg: true,
};

/// (wl_shm format, width, height, stride) from the frame's `buffer` event.
pub type Layout = (u32, u32, u32, u32);

/// wl_shm formats we read: ARGB8888, XRGB8888 (B,G,R,X), ABGR8888, XBGR8888 (R,G,B,X).
/// shortcut: 10-bit formats are refused, add them if a 10-bit output ever shows up.
const FORMATS: [u32; 4] = [0, 1, 0x3432_4241, 0x3432_4258];

pub struct Frame {
    pub id: u32,
    pub layout: Option<Layout>,
    offered: u32,
    pub y_invert: bool,
    pub map: Option<Map>,
    /// The wl_buffer we copied into; can be shown on a surface afterwards.
    pub buf: u32,
    pub ready: bool,
}

impl Frame {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            layout: None,
            offered: 0,
            y_invert: false,
            map: None,
            buf: 0,
            ready: false,
        }
    }

    /// Handle one event of this frame. Err(Failed) when the compositor gave up.
    pub fn event(&mut self, c: &mut Conn, shm: u32, k: &Kind, ev: &Event) -> Result<(), Error> {
        let mut a = ev.args();
        match ev.op {
            op if op == k.buffer => {
                let l = (a.u32()?, a.u32()?, a.u32()?, a.u32()?);
                self.offered = l.0;
                if self.layout.is_none() && FORMATS.contains(&l.0) {
                    self.layout = Some(l);
                }
            }
            op if op == k.buffer_done => self.copy(c, shm, k)?,
            op if op == k.flags => self.y_invert = a.u32()? & proto::frame::Y_INVERT != 0,
            op if op == k.ready => {
                self.ready = true;
                c.send(self.id, proto::frame::DESTROY, &[])?; // done; its buffer stays
            }
            op if op == k.failed => return Err(Error::Failed(String::new())),
            _ => {}
        }
        Ok(())
    }

    fn copy(&mut self, c: &mut Conn, shm: u32, k: &Kind) -> Result<(), Error> {
        let (format, w, h, stride) = self.layout.ok_or(Error::Format(self.offered))?;
        // Sizes come from the compositor: a row must hold w pixels, the pool fit an i32.
        let len = stride
            .checked_mul(h)
            .filter(|&l| l <= i32::MAX as u32 && w > 0);
        let len = match len {
            Some(l) if stride >= w.saturating_mul(4) => l as usize,
            _ => return Err(Error::Format(format)),
        };
        let map = Map::new(len)?;
        let (pool, buf) = (c.new_id(), c.new_id());
        let pool_args = [Arg::U(pool), Arg::Fd(map.fd()), Arg::I(len as i32)];
        c.send(shm, proto::shm::CREATE_POOL, &pool_args)?;
        let (w, h, s) = (w as i32, h as i32, stride as i32);
        let args = [
            Arg::U(buf),
            Arg::I(0),
            Arg::I(w),
            Arg::I(h),
            Arg::I(s),
            Arg::U(format),
        ];
        c.send(pool, proto::shm_pool::CREATE_BUFFER, &args)?;
        c.send(pool, proto::shm_pool::DESTROY, &[])?; // the buffer keeps the memory alive
        match k.ignore_damage_arg {
            true => c.send(self.id, proto::frame::COPY, &[Arg::U(buf), Arg::I(1)])?,
            false => c.send(self.id, proto::frame::COPY, &[Arg::U(buf)])?,
        }
        // Fault our pages in (as huge pages) while the compositor waits for its next
        // frame; else its copy takes ~2000 page faults: ready 21 -> 14 ms median.
        c.flush()?;
        let _ = map.populate(); // only a speed-up: older kernels lack MADV_POPULATE_WRITE
        (self.map, self.buf) = (Some(map), buf);
        Ok(())
    }
}

/// Pump events until every frame is ready; `labels[i]` names frame i in errors.
pub fn wait_all(
    c: &mut Conn,
    shm: u32,
    k: &Kind,
    frames: &mut [Frame],
    labels: &[String],
) -> Result<(), Error> {
    while !frames.iter().all(|f| f.ready) {
        let mut err = None;
        c.pump(|c, ev| {
            let Some(i) = frames.iter().position(|f| f.id == ev.obj) else {
                return Ok(());
            };
            if let Err(e) = frames[i].event(c, shm, k, ev) {
                err.get_or_insert(match e {
                    Error::Failed(_) => Error::Failed(labels[i].clone()),
                    e => e,
                });
            }
            Ok(())
        })?;
        if let Some(e) = err {
            return Err(e);
        }
    }
    Ok(())
}
