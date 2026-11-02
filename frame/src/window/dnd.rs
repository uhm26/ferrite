//! Drag and drop through wl_data_device. Out: [`Window::start_drag`] offers data
//! (e.g. a `text/uri-list` of files) that any app can take. In: [`Drag`] events while
//! something is dragged over us; on a drop the app reads it like a paste
//! ([`Window::drop_receive`]) and then calls [`Window::drop_finish`].
//! Offers share the clipboard's list (both arrive as data_device.data_offer).

use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;

use super::{Event, Window, clipboard};
use crate::sys;
use crate::sys::mem::Pixels;
use crate::wl::proto::{buffer, data_device, data_device_manager, data_offer, data_source};
use crate::wl::proto::{shm_pool, surface};
use crate::wl::{Arg, Error, Event as WlEvent};

pub use data_source::{ACTION_COPY, ACTION_MOVE};

pub enum Drag {
    /// Something offering these mime types is dragged in at (x, y).
    Enter {
        mimes: Vec<String>,
        at: (f64, f64),
    },
    Motion(f64, f64),
    Leave,
    /// Dropped where the last motion was.
    Drop,
    /// Our own drag ended: the action the target took (0 = cancelled or refused).
    Ended(u32),
}

/// What follows the pointer while our drag runs: `w * h` premultiplied ARGB pixels,
/// drawn with the top-left corner at `at` from the pointer.
pub struct DragIcon<'a> {
    pub px: &'a [u32],
    pub w: usize,
    pub h: usize,
    pub at: (i32, i32),
}

#[derive(Default)]
pub(super) struct Dnd {
    /// The offer dragged over us (0 = none), its enter serial, the action chosen.
    pub offer: u32,
    serial: u32,
    action: u32,
    /// Dropped: the offer lives on until the app finishes reading it.
    dropped: bool,
    /// Our drag: its source, the data it serves, the action the target chose.
    source: u32,
    data: Arc<[u8]>,
    source_action: u32,
    /// The drag icon: its surface, buffer and memory (kept until the drag ends).
    icon: Option<(u32, u32, Pixels)>,
}

impl<S: Default> Window<S> {
    /// Start dragging `data` (served for each of `mimes`) out of the window, allowing
    /// `actions` (`ACTION_COPY | ACTION_MOVE`). Call while the left button is held:
    /// the compositor ties the drag to that press.
    /// `icon`, if given, follows the pointer.
    pub fn start_drag(
        &mut self,
        (mimes, data): (&[&str], Vec<u8>),
        actions: u32,
        icon: Option<DragIcon>,
    ) -> Result<(), Error> {
        if self.clip.device == 0 {
            return Ok(());
        }
        self.end_source()?;
        let id = self.conn.new_id();
        let ddm = self.ids.ddm;
        self.conn
            .send(ddm, data_device_manager::CREATE_DATA_SOURCE, &[Arg::U(id)])?;
        for m in mimes {
            self.conn.send(id, data_source::OFFER, &[Arg::S(m)])?;
        }
        self.conn
            .send(id, data_source::SET_ACTIONS, &[Arg::U(actions)])?;
        let icon_surface = match icon {
            Some(_) => self.conn.create_surface(self.ids.compositor)?,
            None => 0,
        };
        let args = [
            Arg::U(id),
            Arg::U(self.surface),
            Arg::U(icon_surface),
            Arg::U(self.input.serial),
        ];
        self.conn
            .send(self.clip.device, data_device::START_DRAG, &args)?;
        self.dnd.source = id;
        self.dnd.data = data.into();
        self.dnd.source_action = 0;
        if let Some(i) = icon {
            self.dnd.icon = Some(self.show_icon(icon_surface, &i)?);
        }
        self.conn.flush()
    }

    /// Put `icon` on surface `s` (start_drag gave it the drag icon role).
    fn show_icon(&mut self, s: u32, icon: &DragIcon) -> Result<(u32, u32, Pixels), Error> {
        let len = icon.w * icon.h * 4;
        let mut pixels = Pixels::new(len)?;
        pixels.as_u32_mut()[..icon.px.len()].copy_from_slice(icon.px);
        let pool = self.conn.create_pool(self.ids.shm, pixels.fd(), len)?;
        let (w, h) = (icon.w as i32, icon.h as i32);
        let buf = self.conn.pool_buffer(pool, 0, w, h)?;
        self.conn.send(pool, shm_pool::DESTROY, &[])?; // the buffer keeps the memory
        // wl_surface v4: attach's offset places the icon relative to the pointer.
        let at = [Arg::U(buf), Arg::I(icon.at.0), Arg::I(icon.at.1)];
        self.conn.send(s, surface::ATTACH, &at)?;
        self.conn
            .send(s, surface::DAMAGE_BUFFER, &[0, 0, w, h].map(Arg::I))?;
        self.conn.send(s, surface::COMMIT, &[])?;
        Ok((s, buf, pixels))
    }

    /// Say whether a drop here would be taken: `mime` (None = no), with which
    /// `actions` and which one preferred. Call on each enter and motion.
    pub fn drag_accept(
        &mut self,
        mime: Option<&str>,
        actions: u32,
        preferred: u32,
    ) -> Result<(), Error> {
        let d = &self.dnd;
        if d.offer == 0 || d.dropped {
            return Ok(());
        }
        let (offer, serial) = (d.offer, d.serial);
        let m = mime.map_or(Arg::U(0), Arg::S); // a null string is length 0
        self.conn
            .send(offer, data_offer::ACCEPT, &[Arg::U(serial), m])?;
        let (actions, preferred) = if mime.is_some() {
            (actions, preferred)
        } else {
            (0, 0)
        };
        self.conn.send(
            offer,
            data_offer::SET_ACTIONS,
            &[Arg::U(actions), Arg::U(preferred)],
        )
    }

    /// The action the compositor settled on for the drag over us (`ACTION_*`).
    pub fn drop_action(&self) -> u32 {
        self.dnd.action
    }

    /// After a drop: a pipe the dragged data arrives on as `mime`.
    pub fn drop_receive(&mut self, mime: &str) -> Result<Option<OwnedFd>, Error> {
        if self.dnd.offer == 0 || !self.dnd.dropped {
            return Ok(None);
        }
        let (read, write) = sys::pipe()?;
        let args = [Arg::S(mime), Arg::Fd(write.as_fd())];
        self.conn.send(self.dnd.offer, data_offer::RECEIVE, &args)?;
        drop(write);
        sys::set_nonblocking(read.as_fd())?;
        Ok(Some(read))
    }

    /// Done with the dropped data (read, or not wanted): tell the source and let go.
    pub fn drop_finish(&mut self) -> Result<(), Error> {
        let d = &mut self.dnd;
        if d.offer == 0 {
            return Ok(());
        }
        let (offer, finish) = (d.offer, d.dropped && d.action != 0);
        (d.offer, d.dropped, d.action) = (0, false, 0);
        if finish {
            self.conn.send(offer, data_offer::FINISH, &[])?;
        }
        self.forget_offer(offer)
    }

    fn forget_offer(&mut self, offer: u32) -> Result<(), Error> {
        self.clip.offers.retain(|(id, _)| *id != offer);
        self.conn.send(offer, data_offer::DESTROY, &[])
    }

    /// Our previous drag source, if any, goes.
    fn end_source(&mut self) -> Result<(), Error> {
        let s = std::mem::take(&mut self.dnd.source);
        self.dnd.data = Arc::from([]);
        if let Some((icon, buf, _pixels)) = self.dnd.icon.take() {
            self.conn.send(icon, surface::DESTROY, &[])?;
            self.conn.send(buf, buffer::DESTROY, &[])?;
        }
        if s != 0 {
            self.conn.send(s, data_source::DESTROY, &[])?;
        }
        Ok(())
    }

    /// data_device enter/leave/motion/drop. Ok(false): not one of those.
    pub(super) fn on_dnd_device(&mut self, ev: &WlEvent) -> Result<bool, Error> {
        let mut a = ev.args();
        let drag = match ev.op {
            data_device::EV_ENTER => {
                let (serial, _surface) = (a.u32()?, a.u32()?);
                let at = (a.fixed()?, a.fixed()?);
                let offer = a.u32()?;
                if self.dnd.offer != 0 && !self.dnd.dropped {
                    self.forget_offer(self.dnd.offer)?;
                }
                let d = &mut self.dnd;
                (d.offer, d.serial, d.action, d.dropped) = (offer, serial, 0, false);
                let mimes = self.clip.offers.iter().find(|(id, _)| *id == offer);
                let mimes = mimes.map(|(_, m)| m.clone()).unwrap_or_default();
                Drag::Enter { mimes, at }
            }
            data_device::EV_MOTION => {
                let _time = a.u32()?;
                Drag::Motion(a.fixed()?, a.fixed()?)
            }
            data_device::EV_LEAVE => {
                if self.dnd.dropped {
                    return Ok(true); // the leave that follows a drop: keep the offer
                }
                let offer = std::mem::take(&mut self.dnd.offer);
                if offer != 0 {
                    self.forget_offer(offer)?;
                }
                Drag::Leave
            }
            data_device::EV_DROP => {
                self.dnd.dropped = true;
                Drag::Drop
            }
            _ => return Ok(false),
        };
        self.events.push_back(Event::Drag(drag));
        Ok(true)
    }

    /// Events for the offer over us or our drag source. Ok(false): neither.
    pub(super) fn on_dnd_object(&mut self, ev: &WlEvent) -> Result<bool, Error> {
        let mut a = ev.args();
        let d = &mut self.dnd;
        if ev.obj == d.offer && d.offer != 0 {
            match ev.op {
                data_offer::EV_ACTION => d.action = a.u32()?,
                data_offer::EV_OFFER => return Ok(false), // mime types: the clipboard list
                _ => {}
            }
            return Ok(true);
        }
        if ev.obj != d.source || d.source == 0 {
            return Ok(false);
        }
        match ev.op {
            data_source::EV_SEND => {
                let _mime = a.str()?;
                let fd = self.conn.take_fd()?;
                let data = Arc::clone(&self.dnd.data);
                std::thread::spawn(move || clipboard::send_all(fd, &data));
            }
            data_source::EV_ACTION => d.source_action = a.u32()?,
            data_source::EV_DND_FINISHED | data_source::EV_CANCELLED => {
                let action = if ev.op == data_source::EV_CANCELLED {
                    0
                } else {
                    d.source_action
                };
                self.end_source()?;
                self.events.push_back(Event::Drag(Drag::Ended(action)));
            }
            _ => {}
        }
        Ok(true)
    }
}
