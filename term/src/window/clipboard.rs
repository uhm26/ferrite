//! Copy and paste through wl_data_device (the regular clipboard). Paste hands the
//! app a pipe to read; copy offers our text and writes it when someone asks.

use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;

use super::Window;
use crate::sys;
use crate::wl::proto::{data_device, data_device_manager, data_offer, data_source};
use crate::wl::{Arg, Error, Event as WlEvent};

/// Text types, most preferred first.
const MIMES: [&str; 4] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
];

#[derive(Default)]
pub(super) struct Clipboard {
    pub device: u32,
    /// Offers the compositor announced, with their mime types.
    offers: Vec<(u32, Vec<String>)>,
    /// The offer that is the current clipboard (0 = empty).
    selection: u32,
    /// Our own data source while we own the clipboard.
    source: u32,
    /// Sources we replaced but the compositor hasn't confirmed gone: a late `send`
    /// for one still carries an fd that must be taken, or the fd queue desyncs.
    retired: Vec<u32>,
    text: Arc<[u8]>,
}

impl Clipboard {
    /// `id` is gone for good (wl_display.delete_id).
    pub fn forget(&mut self, id: u32) {
        self.retired.retain(|&r| r != id);
    }
}

impl Window {
    /// The clipboard holds an image and no text.
    pub fn clipboard_is_image(&self) -> bool {
        let sel = self.clip.selection;
        self.clip.offers.iter().find(|(id, _)| *id == sel).is_some_and(|(_, mimes)| {
            let any = |p: fn(&str) -> bool| mimes.iter().any(|m| p(m));
            any(|m| m.starts_with("image/")) && !any(|m| MIMES.contains(&m))
        })
    }

    /// Ask for the clipboard as text. Returns the pipe to read it from.
    /// With `image`, asks for the image (PNG preferred) instead. Returns the pipe and the
    /// mime type it will carry.
    pub fn paste(&mut self, image: bool) -> Result<Option<(OwnedFd, String)>, Error> {
        let sel = self.clip.selection;
        let Some((_, mimes)) = self.clip.offers.iter().find(|(id, _)| *id == sel) else {
            return Ok(None);
        };
        let mime = if image {
            let png = mimes.iter().find(|m| *m == "image/png");
            png.or_else(|| mimes.iter().find(|m| m.starts_with("image/"))).cloned()
        } else {
            MIMES.iter().find(|m| mimes.iter().any(|x| x == *m)).map(|m| m.to_string())
        };
        let Some(mime) = mime else {
            return Ok(None);
        };
        let mime_arg = mime.clone();
        let (read, write) = sys::pipe()?;
        self.conn.send(
            sel,
            data_offer::RECEIVE,
            &[Arg::S(&mime_arg), Arg::Fd(write.as_fd())],
        )?; // sending an fd flushes at once
        drop(write); // the sender holds its copy; EOF comes when it is done
        sys::set_nonblocking(read.as_fd())?;
        Ok(Some((read, mime)))
    }

    /// Put `text` on the clipboard.
    pub fn copy(&mut self, text: String) -> Result<(), Error> {
        if self.clip.device == 0 || text.is_empty() {
            return Ok(());
        }
        if self.clip.source != 0 {
            self.conn
                .send(self.clip.source, data_source::DESTROY, &[])?;
            self.clip.retired.push(self.clip.source);
        }
        let id = self.conn.new_id();
        self.conn.send(
            self.ids.ddm,
            data_device_manager::CREATE_DATA_SOURCE,
            &[Arg::U(id)],
        )?;
        for m in MIMES {
            self.conn.send(id, data_source::OFFER, &[Arg::S(m)])?;
        }
        let args = [Arg::U(id), Arg::U(self.input.serial)];
        self.conn
            .send(self.clip.device, data_device::SET_SELECTION, &args)?;
        self.clip.source = id;
        self.clip.text = text.into_bytes().into();
        self.conn.flush()
    }

    pub(super) fn on_clipboard(&mut self, ev: &WlEvent) -> Result<(), Error> {
        let mut a = ev.args();
        let c = &mut self.clip;
        if ev.obj == c.device && c.device != 0 {
            match ev.op {
                data_device::EV_DATA_OFFER => c.offers.push((a.u32()?, Vec::new())),
                data_device::EV_SELECTION => {
                    c.selection = a.u32()?;
                    let sel = c.selection;
                    for (id, _) in c.offers.iter().filter(|(id, _)| *id != sel) {
                        self.conn.send(*id, data_offer::DESTROY, &[])?;
                    }
                    self.clip.offers.retain(|(id, _)| *id == sel);
                }
                _ => {} // drag and drop: not supported
            }
        } else if let Some((_, mimes)) = c.offers.iter_mut().find(|(id, _)| *id == ev.obj) {
            if ev.op == data_offer::EV_OFFER {
                mimes.push(a.str()?.to_string());
            }
        } else if ev.obj == c.source && c.source != 0 {
            self.on_source(ev)?;
        } else if c.retired.contains(&ev.obj) && ev.op == data_source::EV_SEND {
            drop(self.conn.take_fd()?); // closing it tells the reader there is nothing
        }
        Ok(())
    }

    fn on_source(&mut self, ev: &WlEvent) -> Result<(), Error> {
        match ev.op {
            data_source::EV_SEND => {
                let fd = self.conn.take_fd()?;
                let text = Arc::clone(&self.clip.text);
                // A slow reader must not stall the terminal: write from a thread.
                std::thread::spawn(move || send_all(fd, &text));
            }
            data_source::EV_CANCELLED => {
                self.conn.send(ev.obj, data_source::DESTROY, &[])?;
                self.clip.source = 0;
                self.clip.text = Arc::from([]);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Write `text` to a paste reader, giving up after 5 s without progress: a reader
/// that stops reading must not keep this thread and the pipe alive forever.
fn send_all(fd: OwnedFd, text: &[u8]) {
    if sys::set_nonblocking(fd.as_fd()).is_err() {
        return;
    }
    let mut f = std::fs::File::from(fd);
    let mut rest = text;
    while !rest.is_empty() {
        match f.write(rest) {
            Ok(n) => rest = &rest[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let mut p = [sys::PollFd::new(f.as_fd(), sys::POLLOUT)];
                if sys::wait(&mut p, 5000).is_err() || !p[0].ready(sys::POLLOUT) {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    #[test]
    fn send_all_delivers_more_than_a_pipe_holds() {
        let (read, write) = crate::sys::pipe().unwrap();
        let text: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        let reader = std::thread::spawn(move || {
            let mut got = Vec::new();
            std::fs::File::from(read).read_to_end(&mut got).unwrap();
            got
        });
        super::send_all(write, &text);
        assert_eq!(reader.join().unwrap(), text);
    }
}
