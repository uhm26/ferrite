//! Connect, bind the data-control manager, then sleep in poll() until the
//! compositor or a pipe has something. Idle costs nothing.

use crate::keeper::Keeper;
use crate::sys::{self, POLLIN, PollFd};
use crate::wl::proto::{self, display, manager, registry};
use crate::wl::{Arg, Conn, Error, Event, Global};

pub fn run() -> Result<(), Error> {
    let mut conn = Conn::connect()?;
    let (mgr, seat) = bind_globals(&mut conn)?;
    let device = conn.new_id();
    let args = [Arg::U(device), Arg::U(seat)];
    conn.send(mgr, manager::GET_DATA_DEVICE, &args)?;
    let mut keeper = Keeper::new(mgr, device);
    let mut fds = Vec::new();
    loop {
        conn.flush()?;
        fds.clear();
        fds.push(PollFd::new(conn.fd(), POLLIN));
        keeper.poll_set(&mut fds);
        sys::wait(&mut fds)?;
        if fds[0].revents != 0 {
            conn.read()?;
        }
        while let Some(ev) = conn.next_event()? {
            if !display_event(&mut conn, &ev)? {
                keeper.event(&mut conn, &ev)?;
            }
        }
        keeper.pump(&mut conn)?;
    }
}

/// wl_display events. Ok(true): it was one.
fn display_event(conn: &mut Conn, ev: &Event) -> Result<bool, Error> {
    if ev.obj != proto::DISPLAY {
        return Ok(false);
    }
    let mut a = ev.args();
    match ev.op {
        display::EV_ERROR => {
            let (object, code) = (a.u32()?, a.u32()?);
            let message = a.str()?.to_owned();
            return Err(Error::Protocol {
                object,
                code,
                message,
            });
        }
        display::EV_DELETE_ID => {
            let id = a.u32()?;
            if id < proto::SERVER_ID_START {
                conn.release_id(id);
            }
        }
        _ => {}
    }
    Ok(true)
}

/// One roundtrip over the registry; bind the data-control manager and a seat.
fn bind_globals(conn: &mut Conn) -> Result<(u32, u32), Error> {
    let reg = conn.get_registry()?;
    let done = conn.sync()?;
    let mut globals = Vec::new();
    let mut synced = false;
    while !synced {
        conn.flush()?;
        sys::wait(&mut [PollFd::new(conn.fd(), POLLIN)])?;
        conn.read()?;
        while let Some(ev) = conn.next_event()? {
            synced |= ev.obj == done;
            if ev.obj == reg && ev.op == registry::EV_GLOBAL {
                let mut a = ev.args();
                let (name, interface) = (a.u32()?, a.str()?.to_owned());
                globals.push(Global {
                    name,
                    interface,
                    version: a.u32()?,
                });
            } else {
                display_event(conn, &ev)?;
            }
        }
    }
    let find = |i: &str| globals.iter().find(|g| g.interface == i);
    let mgr = find(manager::EXT).or_else(|| find(manager::WLR));
    let mgr = mgr.ok_or(Error::Missing("ext-data-control-v1 or wlr-data-control"))?;
    let seat = find("wl_seat").ok_or(Error::Missing("wl_seat"))?;
    Ok((conn.bind(reg, mgr, 1)?, conn.bind(reg, seat, 1)?))
}
