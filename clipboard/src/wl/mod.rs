//! Our own minimal Wayland client: wire format, opcodes, and the setup requests.
//! Trimmed copy of ../term/src/wl/mod.rs (2026-11-01), which came from ../wall.

pub mod proto;
pub mod wire;

pub use wire::{Arg, Conn, Error, Event};

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
}
