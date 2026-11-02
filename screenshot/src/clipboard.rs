//! Put a PNG on the clipboard and serve it until another copy replaces it, like
//! wl-copy does. Uses data-control (ext, else wlr), so no focused window is needed.
//! The source code pattern follows ../clip/src/keeper.rs (2026-11-01).

use std::fs::File;
use std::io::Write;
use std::sync::Arc;

use crate::capture::{Error, Session};
use crate::wl::{self, Arg, proto};
use proto::data_control as dc;

/// Blocks until the selection is taken over (or the compositor goes away).
pub fn serve(s: &mut Session, png: Vec<u8>) -> Result<(), Error> {
    let png: Arc<[u8]> = png.into();
    let mgr = match s.bind(dc::EXT, 1)? {
        Some(m) => m,
        None => s
            .bind(dc::WLR, 1)?
            .ok_or(wl::Error::Missing("ext or wlr data-control"))?,
    };
    let seat = s
        .bind(proto::seat::NAME, 1)?
        .ok_or(wl::Error::Missing(proto::seat::NAME))?;
    let (device, source) = (s.c.new_id(), s.c.new_id());
    s.c.send(mgr, dc::GET_DATA_DEVICE, &[Arg::U(device), Arg::U(seat)])?;
    s.c.send(mgr, dc::CREATE_DATA_SOURCE, &[Arg::U(source)])?;
    s.c.send(source, dc::OFFER, &[Arg::S("image/png")])?;
    s.c.send(device, dc::SET_SELECTION, &[Arg::U(source)])?;
    let mut done = false;
    while !done {
        s.c.pump(|c, ev| {
            if ev.obj != source {
                return Ok(());
            }
            match ev.op {
                dc::EV_SEND => {
                    let (fd, png) = (c.take_fd()?, Arc::clone(&png));
                    // Own thread: a reader that stalls must not block the event loop
                    // (we would never see `cancelled`). Exiting kills stuck writers.
                    // The reader may close early (EPIPE); that is its business.
                    std::thread::spawn(move || File::from(fd).write_all(&png));
                }
                dc::EV_CANCELLED => done = true,
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(())
}
