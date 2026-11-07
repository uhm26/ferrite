//! The clipboard state machine. Every copy's text and image are read in at once;
//! when the selection goes empty (the copying app closed), we offer them ourselves.

use std::io;
use std::os::fd::AsFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::pipes::{Kept, Reader, STALL, Writer};
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
/// The image type nearly every app pastes; preferred over any other image type.
const PNG: &str = "image/png";
/// Our own sources carry this, so we can tell them apart from real copies.
const MARKER: &str = "application/x-clipboard-keeper";
/// Password managers mark secrets with this (KeePassXC, KDE); never keep those.
const SECRET: &str = "x-kde-passwordManagerHint";
/// Pastes served at once; past this the oldest (likely stuck) one is dropped.
/// Each costs one fd (the data is shared), and the soft fd limit is 1024.
const MAX_WRITERS: usize = 256;
/// How long the selection must stay empty before we take over. Apps that replace
/// their copy destroy the old source first (empty for a moment), then set the new
/// one; taking over at once raced that and wiped the fresh copy.
const GRACE: Duration = Duration::from_millis(100);

/// What one data offer advertises, as far as we care.
#[derive(Debug)]
pub struct Offer {
    id: u32,
    /// Index into TEXT of the best text type offered; TEXT.len() = none.
    best: usize,
    /// PNG if offered, else the first image type.
    image: Option<String>,
    ours: bool,
    secret: bool,
}

impl Offer {
    pub fn new(id: u32) -> Self {
        let best = TEXT.len();
        Self {
            id,
            best,
            image: None,
            ours: false,
            secret: false,
        }
    }

    pub fn add_mime(&mut self, mime: &str) {
        match mime {
            MARKER => self.ours = true,
            SECRET => self.secret = true,
            m if m.starts_with("image/") => {
                if self.image.is_none() || m == PNG {
                    self.image = Some(m.to_owned());
                }
            }
            m => {
                let rank = TEXT.iter().position(|t| *t == m).unwrap_or(TEXT.len());
                self.best = self.best.min(rank);
            }
        }
    }

    /// The types to read (best text, then the image), if this is a copy to keep.
    pub fn wanted(&self) -> Vec<&str> {
        if self.ours || self.secret {
            return Vec::new();
        }
        let text = TEXT.get(self.best).copied();
        text.into_iter().chain(self.image.as_deref()).collect()
    }
}

pub struct Keeper {
    manager: u32,
    device: u32,
    /// Offers announced but not yet tied to a selection.
    offers: Vec<Offer>,
    /// The last copy as (type we serve it as, bytes); empty = nothing to paste.
    data: Vec<(String, Rc<Kept>)>,
    /// Our live source while we own the selection.
    source: Option<u32>,
    /// Since when the selection is empty; None = something owns it.
    empty_since: Option<Instant>,
    /// Parts of the last copy still coming in: (type asked for, reader).
    readers: Vec<(String, Reader)>,
    writers: Vec<Writer>,
}

impl Keeper {
    pub fn new(manager: u32, device: u32) -> Self {
        Self {
            manager,
            device,
            offers: Vec::new(),
            data: Vec::new(),
            source: None,
            empty_since: None,
            readers: Vec::new(),
            writers: Vec::new(),
        }
    }

    /// The pipes we wait on, after the Wayland socket.
    pub fn poll_set(&self, fds: &mut Vec<PollFd>) {
        fds.extend(
            self.readers
                .iter()
                .map(|(_, r)| PollFd::new(r.fd(), POLLIN)),
        );
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
                source::EV_SEND => {
                    let fd = conn.take_fd()?; // first: it is queued whatever the mime says
                    if let Ok(mime) = args.str() {
                        self.serve(mime, fd); // a non-UTF-8 mime gets an empty paste
                    }
                }
                source::EV_CANCELLED => {
                    conn.send(ev.obj, source::DESTROY, &[])?;
                    self.source = None;
                }
                _ => {}
            }
        } else if ev.op == offer::EV_OFFER
            && let Some(o) = self.offers.iter_mut().find(|o| o.id == ev.obj)
            && let Ok(mime) = args.str()
        {
            o.add_mime(mime); // a non-UTF-8 mime from some app is skipped, not fatal
        }
        Ok(())
    }

    /// How long poll may sleep: forever, unless a copy is still coming in or the
    /// grace before a take-over is running.
    pub fn timeout_ms(&self) -> i32 {
        let now = Instant::now();
        // A passed grace deadline is left out, or poll would spin while we wait on a reader.
        let grace = self.empty_since.map(|t| t + GRACE).filter(|d| *d > now);
        let readers = self.readers.iter().map(|(_, r)| r.deadline());
        let Some(next) = readers.chain(grace).min() else {
            return -1;
        };
        let left = next.saturating_duration_since(now).as_millis();
        i32::try_from(left).unwrap_or(i32::MAX).saturating_add(1) // wake just after it
    }

    /// Move pipe transfers along; call after every wakeup.
    /// shortcut: steps every transfer, not only ready ones; fine until hundreds of
    /// pastes are stuck at once.
    pub fn pump(&mut self, conn: &mut Conn) -> Result<(), Error> {
        let now = Instant::now();
        let mut i = 0;
        while i < self.readers.len() {
            let step = self.readers[i].1.step();
            let stalled = self.readers[i].1.deadline() <= now;
            if matches!(step, Ok(false)) && !stalled {
                i += 1;
                continue;
            }
            let (mime, reader) = self.readers.swap_remove(i);
            match step {
                Ok(true) => self.keep(&mime, reader),
                Ok(false) => eprintln!("clipboard: gave up on {mime}: no data for {STALL:?}"),
                Err(e) => eprintln!("clipboard: reading {mime} failed: {e}"),
            }
        }
        self.writers.retain_mut(|w| match w.step() {
            Ok(done) => !done,
            Err(e) => {
                if e.kind() != io::ErrorKind::BrokenPipe {
                    eprintln!("clipboard: a paste failed: {e}"); // the app closing early is normal
                }
                false
            }
        });
        self.take_over(conn) // the selection may have emptied while we read
    }

    /// Store a finished part under every type it can be pasted as.
    fn keep(&mut self, mime: &str, reader: Reader) {
        let Some(kept) = reader.finish() else {
            return;
        };
        let kept = Rc::new(kept);
        let names: &[&str] = if TEXT.contains(&mime) { &TEXT } else { &[mime] };
        let parts = names.iter().map(|n| (n.to_string(), kept.clone()));
        self.data.extend(parts);
    }

    fn selection(&mut self, conn: &mut Conn, id: u32) -> Result<(), Error> {
        if id == 0 {
            self.empty_since = Some(Instant::now()); // pump takes over after GRACE
            return Ok(());
        }
        self.empty_since = None;
        let Some(offer) = self.take_offer(id) else {
            return Ok(());
        };
        if !offer.ours {
            // A new copy: what we kept is stale now, even if we can't keep this one.
            self.data.clear();
            self.readers.clear();
        }
        for mime in offer.wanted() {
            let (r, w) = sys::pipe()?;
            conn.send(id, offer::RECEIVE, &[Arg::S(mime), Arg::Fd(w.as_fd())])?;
            drop(w); // the app holds its own copy; ours would hide EOF
            self.readers.push((mime.to_owned(), Reader::new(r)?));
        }
        conn.send(id, offer::DESTROY, &[]) // the transfer goes on without it
    }

    /// Forget an announced offer; the caller gets what it advertised.
    fn take_offer(&mut self, id: u32) -> Option<Offer> {
        let i = self.offers.iter().position(|o| o.id == id)?;
        Some(self.offers.swap_remove(i))
    }

    /// Own the selection with the kept copy, if it has been empty for GRACE.
    fn take_over(&mut self, conn: &mut Conn) -> Result<(), Error> {
        let busy = self.source.is_some() || !self.readers.is_empty();
        let settled = self.empty_since.is_some_and(|t| t.elapsed() >= GRACE);
        if !settled || busy || self.data.is_empty() {
            return Ok(());
        }
        let id = conn.new_id();
        conn.send(self.manager, manager::CREATE_DATA_SOURCE, &[Arg::U(id)])?;
        for mime in self.data.iter().map(|(m, _)| m.as_str()).chain([MARKER]) {
            conn.send(id, source::OFFER, &[Arg::S(mime)])?;
        }
        conn.send(self.device, device::SET_SELECTION, &[Arg::U(id)])?;
        self.source = Some(id);
        self.empty_since = None; // until the compositor says so; stale would re-take
        Ok(())
    }

    /// A paste asked for a type of our copy: start writing it.
    fn serve(&mut self, mime: &str, fd: std::os::fd::OwnedFd) {
        let Some((_, data)) = self.data.iter().find(|(m, _)| m == mime) else {
            return; // the marker or an unknown type: dropping fd = an empty paste
        };
        let data = data.clone();
        if self.writers.len() >= MAX_WRITERS {
            self.writers.remove(0);
        }
        match Writer::new(fd, data) {
            Ok(w) => self.writers.push(w),
            Err(e) => eprintln!("clipboard: serving a paste failed: {e}"),
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
        assert_eq!(o.wanted(), ["text/plain;charset=utf-8"]);
        assert_eq!(offer(&["TEXT", "UTF8_STRING"]).wanted(), ["UTF8_STRING"]);
    }

    #[test]
    fn picks_png_else_the_first_image() {
        let o = offer(&["image/jpeg", "text/plain", PNG, "image/bmp"]);
        assert_eq!(o.wanted(), ["text/plain", PNG]);
        assert_eq!(offer(&["image/webp", "image/bmp"]).wanted(), ["image/webp"]);
    }

    #[test]
    fn skips_other_types_secrets_and_our_own() {
        assert!(offer(&["text/html", "application/pdf"]).wanted().is_empty());
        assert!(offer(&["text/plain", PNG, SECRET]).wanted().is_empty());
        assert!(offer(&["text/plain", PNG, MARKER]).wanted().is_empty());
    }

    fn finished_reader(bytes: &[u8]) -> Reader {
        use std::io::Write;
        let (r, w) = sys::pipe().unwrap();
        std::fs::File::from(w).write_all(bytes).unwrap();
        let mut reader = Reader::new(r).unwrap();
        assert!(reader.step().unwrap());
        reader
    }

    #[test]
    fn keeps_text_under_every_text_name_and_images_as_is() {
        let mut k = Keeper::new(2, 3);
        k.keep("UTF8_STRING", finished_reader(b"hi"));
        k.keep(PNG, finished_reader(b"\x89PNG"));
        k.keep("image/jpeg", finished_reader(b"")); // empty part: not kept
        let names: Vec<&str> = k.data.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(names, [TEXT.as_slice(), &[PNG]].concat());
    }

    #[test]
    fn sleeps_forever_unless_a_copy_is_coming_in() {
        let mut k = Keeper::new(2, 3);
        assert_eq!(k.timeout_ms(), -1);
        let (r, _w) = sys::pipe().unwrap();
        k.readers.push((PNG.into(), Reader::new(r).unwrap()));
        let ms = k.timeout_ms();
        assert!(ms > 9_000 && ms <= STALL.as_millis() as i32 + 1, "{ms}");
    }

    #[test]
    fn wakes_once_when_the_grace_ends_then_sleeps() {
        let mut k = Keeper::new(2, 3);
        k.empty_since = Some(Instant::now());
        let ms = k.timeout_ms();
        assert!(ms > 50 && ms <= GRACE.as_millis() as i32 + 1, "{ms}");
        k.empty_since = Some(Instant::now() - GRACE); // grace over, nothing to wait for
        assert_eq!(k.timeout_ms(), -1);
    }
}
