//! The clipboard state machine. Every text copy is read in at once; when the
//! selection goes empty (the copying app closed), we offer the kept copy ourselves.

use std::os::fd::AsFd;
use std::rc::Rc;

use crate::pipes::{Kept, Reader, Writer};
use crate::sys::{self, POLLIN, POLLOUT, PollFd};
use crate::wl::proto::{device, manager, offer, source};
use crate::wl::{Arg, Conn, Error, Event};

/// Text mime types we take (best first) and offer back, all with the same bytes.
pub const TEXT: [&str; 5] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];
/// Our own sources carry this, so we can tell them apart from real copies.
const MARKER: &str = "application/x-clip-keeper";
/// Password managers mark secrets with this (KeePassXC, KDE); never keep those.
const SECRET: &str = "x-kde-passwordManagerHint";
/// Pastes served at once; past this the oldest (likely stuck) one is dropped.
/// Each costs one fd (the data is shared), and the soft fd limit is 1024.
const MAX_WRITERS: usize = 256;

/// What one data offer advertises, as far as we care.
#[derive(Clone, Copy, Debug)]
pub struct Offer {
    id: u32,
    /// Index into TEXT of the best text type offered; TEXT.len() = none.
    best: usize,
    ours: bool,
    secret: bool,
}

impl Offer {
    pub fn new(id: u32) -> Self {
        let best = TEXT.len();
        Self {
            id,
            best,
            ours: false,
            secret: false,
        }
    }

    pub fn add_mime(&mut self, mime: &str) {
        match mime {
            MARKER => self.ours = true,
            SECRET => self.secret = true,
            m => {
                let rank = TEXT.iter().position(|t| *t == m).unwrap_or(TEXT.len());
                self.best = self.best.min(rank);
            }
        }
    }

    /// The text type to read, if this is a copy we should keep.
    pub fn wanted(&self) -> Option<&'static str> {
        let keep = !self.ours && !self.secret;
        TEXT.get(self.best).copied().filter(|_| keep)
    }
}

pub struct Keeper {
    manager: u32,
    device: u32,
    /// Offers announced but not yet tied to a selection.
    offers: Vec<Offer>,
    /// The last text copy, if it is still the right thing to paste.
    data: Option<Rc<Kept>>,
    /// Our live source while we own the selection.
    source: Option<u32>,
    /// The selection is currently empty.
    empty: bool,
    reader: Option<Reader>,
    writers: Vec<Writer>,
}

impl Keeper {
    pub fn new(manager: u32, device: u32) -> Self {
        Self {
            manager,
            device,
            offers: Vec::new(),
            data: None,
            source: None,
            empty: false,
            reader: None,
            writers: Vec::new(),
        }
    }

    /// The pipes we wait on, after the Wayland socket.
    pub fn poll_set(&self, fds: &mut Vec<PollFd>) {
        fds.extend(self.reader.iter().map(|r| PollFd::new(r.fd(), POLLIN)));
        fds.extend(self.writers.iter().map(|w| PollFd::new(w.fd(), POLLOUT)));
    }

    pub fn event(&mut self, conn: &mut Conn, ev: &Event) -> Result<(), Error> {
        let mut args = ev.args();
        if ev.obj == self.device {
            match ev.op {
                device::EV_DATA_OFFER => self.offers.push(Offer::new(args.u32()?)),
                device::EV_SELECTION => self.selection(conn, args.u32()?)?,
                device::EV_PRIMARY_SELECTION => {
                    let id = args.u32()?; // we only keep the regular selection
                    if self.take_offer(id).is_some() {
                        conn.send(id, offer::DESTROY, &[])?;
                    }
                }
                device::EV_FINISHED => return Err(Error::Missing("a seat")),
                _ => {}
            }
        } else if Some(ev.obj) == self.source {
            match ev.op {
                source::EV_SEND => self.serve(args.str()?, conn.take_fd()?),
                source::EV_CANCELLED => {
                    conn.send(ev.obj, source::DESTROY, &[])?;
                    self.source = None;
                }
                _ => {}
            }
        } else if ev.op == offer::EV_OFFER
            && let Some(o) = self.offers.iter_mut().find(|o| o.id == ev.obj)
        {
            o.add_mime(args.str()?);
        }
        Ok(())
    }

    /// Move pipe transfers along; call after every wakeup.
    pub fn pump(&mut self, conn: &mut Conn) -> Result<(), Error> {
        if let Some(reader) = &mut self.reader {
            match reader.step() {
                Ok(false) => {}
                Ok(true) => {
                    self.data = self.reader.take().and_then(Reader::finish).map(Rc::new);
                    self.take_over(conn)?;
                }
                Err(e) => {
                    eprintln!("clip: reading a copy failed: {e}");
                    self.reader = None;
                }
            }
        }
        self.writers.retain_mut(|w| matches!(w.step(), Ok(false)));
        Ok(())
    }

    fn selection(&mut self, conn: &mut Conn, id: u32) -> Result<(), Error> {
        self.empty = id == 0;
        if self.empty {
            return self.take_over(conn);
        }
        let Some(offer) = self.take_offer(id) else {
            return Ok(());
        };
        if !offer.ours {
            // A new copy: what we kept is stale now, even if we can't keep this one.
            self.data = None;
            self.reader = None;
        }
        if let Some(mime) = offer.wanted() {
            let (r, w) = sys::pipe()?;
            conn.send(id, offer::RECEIVE, &[Arg::S(mime), Arg::Fd(w.as_fd())])?;
            drop(w); // the app holds its own copy; ours would hide EOF
            self.reader = Some(Reader::new(r)?);
        }
        conn.send(id, offer::DESTROY, &[]) // the transfer goes on without it
    }

    /// Forget an announced offer; the caller gets what it advertised.
    fn take_offer(&mut self, id: u32) -> Option<Offer> {
        let i = self.offers.iter().position(|o| o.id == id)?;
        Some(self.offers.swap_remove(i))
    }

    /// Own the selection with the kept copy, if it is empty and we have one.
    fn take_over(&mut self, conn: &mut Conn) -> Result<(), Error> {
        let busy = self.source.is_some() || self.reader.is_some();
        if !self.empty || busy || self.data.is_none() {
            return Ok(());
        }
        let id = conn.new_id();
        conn.send(self.manager, manager::CREATE_DATA_SOURCE, &[Arg::U(id)])?;
        for mime in TEXT.iter().chain([&MARKER]) {
            conn.send(id, source::OFFER, &[Arg::S(mime)])?;
        }
        conn.send(self.device, device::SET_SELECTION, &[Arg::U(id)])?;
        self.source = Some(id);
        Ok(())
    }

    /// A paste asked for our copy: start writing it (the marker type gets nothing).
    fn serve(&mut self, mime: &str, fd: std::os::fd::OwnedFd) {
        let Some(data) = self.data.clone().filter(|_| mime != MARKER) else {
            return; // dropping fd closes the pipe: an empty paste
        };
        if self.writers.len() >= MAX_WRITERS {
            self.writers.remove(0);
        }
        match Writer::new(fd, data) {
            Ok(w) => self.writers.push(w),
            Err(e) => eprintln!("clip: serving a paste failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(mimes: &[&str]) -> Offer {
        let mut o = Offer::new(0xff00_0001);
        mimes.iter().for_each(|m| o.add_mime(m));
        o
    }

    #[test]
    fn picks_the_best_text_type() {
        let o = offer(&["text/html", "STRING", "text/plain;charset=utf-8", "TEXT"]);
        assert_eq!(o.wanted(), Some("text/plain;charset=utf-8"));
        assert_eq!(
            offer(&["TEXT", "UTF8_STRING"]).wanted(),
            Some("UTF8_STRING")
        );
    }

    #[test]
    fn skips_images_secrets_and_our_own() {
        assert_eq!(offer(&["image/png"]).wanted(), None);
        assert_eq!(offer(&["text/plain", SECRET]).wanted(), None);
        assert_eq!(offer(&["text/plain", MARKER]).wanted(), None);
    }
}
