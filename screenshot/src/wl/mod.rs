//! Our own minimal Wayland client: wire format, opcodes, and the setup requests.
//! Trimmed copy of ../clip/src/wl/mod.rs (2026-11-01), which came from ../term and ../wall.

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

    /// Flush, then hand every waiting event to `f`; if none is waiting, block in poll
    /// until some arrive. Callers loop on this until their state says done.
    pub fn pump(
        &mut self,
        mut f: impl FnMut(&mut Self, &Event) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.flush()?;
        loop {
            let mut any = false;
            while let Some(ev) = self.next_event()? {
                if ev.obj == proto::DISPLAY && ev.op == proto::display::EV_ERROR {
                    return Err(protocol_error(&ev));
                }
                f(self, &ev)?;
                any = true;
            }
            if any {
                return Ok(());
            }
            crate::sys::wait_readable(self.fd())?;
            self.read()?;
        }
    }
}

fn protocol_error(ev: &Event) -> Error {
    let mut a = ev.args();
    match (a.u32(), a.u32(), a.str()) {
        (Ok(object), Ok(code), Ok(m)) => Error::Protocol {
            object,
            code,
            message: m.to_owned(),
        },
        _ => Error::Malformed,
    }
}
