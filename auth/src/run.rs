//! The daemon loop: polkit calls, the open popup and its helper, one request at a time.

use crate::agent::{self, Event, FAILED, Request};
use crate::session::{Outcome, Session};
use frame::dbus::Conn;
use frame::sys::{self, POLLIN, PollFd};
use std::collections::VecDeque;
use std::os::fd::BorrowedFd;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Agent {
    active: Option<Session>,
    queue: VecDeque<Box<Request>>,
}

pub fn run() -> Res<()> {
    let session = std::env::var("XDG_SESSION_ID").map_err(|_| "XDG_SESSION_ID is not set")?;
    let mut conn = Conn::system()?;
    agent::register(&mut conn, &session)?;
    eprintln!("auth: agent of session {session}");
    let mut a = Agent::default();
    loop {
        for msg in conn.read_ready()? {
            match agent::handle(&mut conn, msg)? {
                Some(Event::Begin(req)) => a.queue.push_back(req),
                Some(Event::Cancel { cookie }) => a.cancel(&mut conn, &cookie)?,
                None => {}
            }
        }
        a.next(&mut conn)?;
        a.wait(&mut conn)?;
    }
}

impl Agent {
    /// Opens the next queued request if none is open.
    fn next(&mut self, conn: &mut Conn) -> Res<()> {
        while self.active.is_none() {
            let Some(req) = self.queue.pop_front() else {
                return Ok(());
            };
            match Session::start(req) {
                Ok(s) => self.active = Some(s),
                Err((req, why)) => {
                    eprintln!("auth: {}: {why}", req.action);
                    conn.reply_error(&req.call, FAILED, &why)?;
                }
            }
        }
        if let Some(s) = &mut self.active {
            // First frame of a new window (later ones come from events).
            if let Some(out) = s.on_window(false) {
                self.finish(conn, out)?;
            }
        }
        Ok(())
    }

    fn cancel(&mut self, conn: &mut Conn, cookie: &str) -> Res<()> {
        if self.active.as_ref().is_some_and(|s| s.req.cookie == cookie) {
            self.finish(conn, Err((agent::CANCELLED, "cancelled by polkit".into())))?;
        }
        while let Some(i) = self.queue.iter().position(|r| r.cookie == cookie) {
            if let Some(req) = self.queue.remove(i) {
                conn.reply_error(&req.call, agent::CANCELLED, "cancelled by polkit")?;
            }
        }
        Ok(())
    }

    /// Answers polkit and closes the popup (dropping it wipes the typed text).
    fn finish(&mut self, conn: &mut Conn, out: Outcome) -> Res<()> {
        let Some(s) = self.active.take() else {
            return Ok(());
        };
        match out {
            Ok(()) => conn.reply(&s.req.call, &[])?,
            Err((name, text)) => {
                eprintln!("auth: {}: {text}", s.req.action);
                conn.reply_error(&s.req.call, name, &text)?;
            }
        }
        Ok(())
    }

    /// Sleeps until polkit, the window or the helper has something (0 CPU when idle).
    fn wait(&mut self, conn: &mut Conn) -> Res<()> {
        // SAFETY: the fd belongs to `conn`, which outlives this borrow.
        let bus = unsafe { BorrowedFd::borrow_raw(conn.fd()) };
        let mut fds = vec![PollFd::new(bus, POLLIN)];
        let mut timeout = -1;
        if let Some(s) = &mut self.active {
            s.prompt.flush()?;
            timeout = s.prompt.timeout();
            fds.push(s.prompt.poll_fd());
            fds.extend(s.helper_fd());
        }
        sys::wait(&mut fds, timeout)?;
        let hit = |i: usize| fds.get(i).is_some_and(|f| f.ready(!0));
        let (win, helper) = (hit(1), hit(2));
        let Some(s) = &mut self.active else {
            return Ok(());
        };
        let mut out = if helper { s.on_helper() } else { None };
        if out.is_none() {
            out = s.on_window(win);
        }
        match out {
            Some(o) => self.finish(conn, o),
            None => Ok(()),
        }
    }
}
