//! Our own minimal Wayland client: wire format, opcodes, and typed requests for the
//! objects a wallpaper needs (registry, compositor, shm, surface, output, layer shell).

pub mod proto;
pub mod wire;

pub use wire::{Arg, Conn, Error, Event};

use proto::{buffer, compositor, layer_shell, layer_surface, region, shm, shm_pool, surface};
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

    /// Background-layer surface stretched over the whole `output`.
    pub fn background_layer(&mut self, shell: u32, surf: u32, output: u32) -> Result<u32, Error> {
        let id = self.new_id();
        let layer = Arg::U(layer_shell::LAYER_BACKGROUND);
        let args = [
            Arg::U(id),
            Arg::U(surf),
            Arg::U(output),
            layer,
            Arg::S("wall"),
        ];
        self.send(shell, layer_shell::GET_LAYER_SURFACE, &args)?;
        self.send(
            id,
            layer_surface::SET_ANCHOR,
            &[Arg::U(layer_surface::ANCHOR_ALL)],
        )?;
        // -1: cover the whole output, also under bars that reserve space.
        self.send(id, layer_surface::SET_EXCLUSIVE_ZONE, &[Arg::I(-1)])?;
        self.send(surf, surface::COMMIT, &[])?;
        Ok(id)
    }

    pub fn ack_configure(&mut self, layer: u32, serial: u32) -> Result<(), Error> {
        self.send(layer, layer_surface::ACK_CONFIGURE, &[Arg::U(serial)])
    }

    /// One XRGB8888 buffer over a whole shm file. The pool is destroyed at once:
    /// its memory lives on until the buffer is destroyed (wl_shm_pool.destroy).
    pub fn shm_buffer(
        &mut self,
        shm_id: u32,
        fd: BorrowedFd,
        w: i32,
        h: i32,
    ) -> Result<u32, Error> {
        let pool = self.new_id();
        self.send(
            shm_id,
            shm::CREATE_POOL,
            &[Arg::U(pool), Arg::Fd(fd), Arg::I(w * h * 4)],
        )?;
        let buf = self.new_id();
        let fmt = Arg::U(shm::FORMAT_XRGB8888);
        let args = [
            Arg::U(buf),
            Arg::I(0),
            Arg::I(w),
            Arg::I(h),
            Arg::I(w * 4),
            fmt,
        ];
        self.send(pool, shm_pool::CREATE_BUFFER, &args)?;
        self.send(pool, shm_pool::DESTROY, &[])?;
        Ok(buf)
    }

    /// Show `buf` on `surf`: scale, full damage, fully opaque, commit.
    pub fn present(&mut self, comp: u32, surf: u32, buf: u32, scale: i32) -> Result<(), Error> {
        let (w, h) = (i32::MAX, i32::MAX);
        let reg = self.new_id();
        self.send(comp, compositor::CREATE_REGION, &[Arg::U(reg)])?;
        self.send(
            reg,
            region::ADD,
            &[Arg::I(0), Arg::I(0), Arg::I(w), Arg::I(h)],
        )?;
        self.send(surf, surface::SET_OPAQUE_REGION, &[Arg::U(reg)])?;
        self.send(reg, region::DESTROY, &[])?;
        self.send(surf, surface::SET_BUFFER_SCALE, &[Arg::I(scale)])?;
        self.send(surf, surface::ATTACH, &[Arg::U(buf), Arg::I(0), Arg::I(0)])?;
        self.send(
            surf,
            surface::DAMAGE_BUFFER,
            &[Arg::I(0), Arg::I(0), Arg::I(w), Arg::I(h)],
        )?;
        self.send(surf, surface::COMMIT, &[])
    }

    pub fn destroy_buffer(&mut self, buf: u32) -> Result<(), Error> {
        self.send(buf, buffer::DESTROY, &[])
    }

    pub fn destroy_layer(&mut self, layer: u32, surf: u32) -> Result<(), Error> {
        self.send(layer, layer_surface::DESTROY, &[])?;
        self.send(surf, surface::DESTROY, &[])
    }

    pub fn release_output(&mut self, output: u32) -> Result<(), Error> {
        self.send(output, proto::output::RELEASE, &[])
    }
}
