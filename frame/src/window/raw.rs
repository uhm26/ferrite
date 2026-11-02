//! Room for objects the window doesn't know: an app binds a global, makes its own
//! objects on the window's connection (a video subsurface and its dmabuf buffers),
//! claims their ids, and reads their events back from [`Window::next_foreign`].
//! Added 2026-11-02 for vid.

use super::Window;
use crate::wl::proto::toplevel;
use crate::wl::{Arg, Conn, Error, Event as WlEvent};

impl<S> Window<S> {
    /// The connection, for requests frame has no method for.
    pub fn conn(&mut self) -> &mut Conn {
        &mut self.conn
    }

    /// The window's own wl_surface (parent for subsurfaces).
    pub fn surface_id(&self) -> u32 {
        self.surface
    }

    pub fn compositor_id(&self) -> u32 {
        self.ids.compositor
    }

    /// Bind the first global named `interface` at `min(its version, max)`; None if the
    /// compositor has none. Returns (object id, bound version).
    pub fn bind_global(&mut self, interface: &str, max: u32) -> Result<Option<(u32, u32)>, Error> {
        let Some(g) = self
            .globals
            .iter()
            .find(|g| g.interface == interface)
            .cloned()
        else {
            return Ok(None);
        };
        let id = self.conn.bind(self.ids.registry, &g, max)?;
        Ok(Some((id, g.version.min(max))))
    }

    /// Events for `id` go to [`Self::next_foreign`] from now on, until the
    /// compositor deletes it.
    pub fn claim(&mut self, id: u32) {
        self.claimed.push(id);
    }

    pub fn next_foreign(&mut self) -> Option<WlEvent> {
        self.foreign.pop_front()
    }

    /// Ask for fullscreen (on the current output) or back to a window. The answer
    /// arrives as a configure; [`Self::fullscreen`] follows it.
    pub fn set_fullscreen(&mut self, on: bool) -> Result<(), Error> {
        if on {
            self.conn
                .send(self.toplevel, toplevel::SET_FULLSCREEN, &[Arg::U(0)])
        } else {
            self.conn
                .send(self.toplevel, toplevel::UNSET_FULLSCREEN, &[])
        }
    }
}
