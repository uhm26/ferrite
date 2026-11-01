//! Our own minimal Wayland client: wire format, opcodes, and typed requests.
//! Started as a copy of ../wall/src/wl/mod.rs (2026-11-01).

pub mod proto;
pub mod wire;

pub use wire::{Arg, Conn, Error, Event};

use proto::{compositor, data_device_manager, region, shm, shm_pool, surface};
use std::os::fd::BorrowedFd;

/// A global the compositor announced (wl_registry.global).
#[derive(Clone, Debug)]
pub struct Global {
    pub name: u32,
    pub interface: String,
    pub version: u32,
}

impl Conn {
    pub fn get_registry(&mut self) -> Result<u32, Error> {
        let id = self.new_id();
        self.send(proto::DISPLAY, proto::display::GET_REGISTRY, &[Arg::U(id)])?;
        Ok(id)
    }

    pub fn sync(&mut self) -> Result<u32, Error> {
        let id = self.new_id();
        self.send(proto::DISPLAY, proto::display::SYNC, &[Arg::U(id)])?;
        Ok(id)
    }

    /// Bind a global at `min(version, max)`.
    pub fn bind(&mut self, registry: u32, g: &Global, max: u32) -> Result<u32, Error> {
        let id = self.new_id();
        let v = g.version.min(max);
        let args = [Arg::U(g.name), Arg::S(&g.interface), Arg::U(v), Arg::U(id)];
        self.send(registry, proto::registry::BIND, &args)?;
        Ok(id)
    }

    pub fn create_surface(&mut self, comp: u32) -> Result<u32, Error> {
        let id = self.new_id();
        self.send(comp, compositor::CREATE_SURFACE, &[Arg::U(id)])?;
        Ok(id)
    }

    /// Mark the whole surface opaque, so the compositor skips what is behind it.
    pub fn opaque_everywhere(&mut self, comp: u32, surf: u32) -> Result<(), Error> {
        let reg = self.new_id();
        self.send(comp, compositor::CREATE_REGION, &[Arg::U(reg)])?;
        let all = [0, 0, i32::MAX, i32::MAX].map(Arg::I);
        self.send(reg, region::ADD, &all)?;
        self.send(surf, surface::SET_OPAQUE_REGION, &[Arg::U(reg)])?;
        self.send(reg, region::DESTROY, &[])
    }

    pub fn data_device(&mut self, manager: u32, seat: u32) -> Result<u32, Error> {
        let id = self.new_id();
        let args = [Arg::U(id), Arg::U(seat)];
        self.send(manager, data_device_manager::GET_DATA_DEVICE, &args)?;
        Ok(id)
    }

    /// A pool over `len` bytes of a shm file.
    pub fn create_pool(&mut self, shm_id: u32, fd: BorrowedFd, len: usize) -> Result<u32, Error> {
        let pool = self.new_id();
        let args = [Arg::U(pool), Arg::Fd(fd), Arg::I(len as i32)];
        self.send(shm_id, shm::CREATE_POOL, &args)?;
        Ok(pool)
    }

    /// One ARGB8888 (premultiplied) buffer at `offset` in `pool`.
    pub fn pool_buffer(&mut self, pool: u32, offset: usize, w: i32, h: i32) -> Result<u32, Error> {
        let buf = self.new_id();
        let fmt = Arg::U(shm::FORMAT_ARGB8888);
        let args = [Arg::U(buf), Arg::I(offset as i32), Arg::I(w), Arg::I(h), Arg::I(w * 4), fmt];
        self.send(pool, shm_pool::CREATE_BUFFER, &args)?;
        Ok(buf)
    }
}
