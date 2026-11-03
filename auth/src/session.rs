//! One request from start to answer: the helper conversation behind the popup.
//! A wrong password shows an error and starts a fresh helper (same cookie), like other
//! agents; Esc, Cancel or closing the window cancels.

use crate::agent::{CANCELLED, FAILED, Request};
use crate::helper::{self, Helper, Line};
use crate::prompt::{Action, Prompt, View};
use crate::user;
use frame::sys::{POLLIN, PollFd};

/// How a request ended: `Ok` = authenticated, `Err((error name, text))` for polkit.
pub type Outcome = Result<(), (&'static str, String)>;

pub struct Session {
    pub req: Box<Request>,
    user: String,
    helper: Option<Helper>,
    /// PAM said something wrong during this attempt (shown instead of our own text).
    pam_error: bool,
    pub prompt: Prompt,
}

impl Session {
    pub fn start(req: Box<Request>) -> Result<Session, (Box<Request>, String)> {
        let Some(user) = user::pick(&req.identities).and_then(user::name) else {
            return Err((req, "no user polkit allows has a name".into()));
        };
        let asker = match &req.program {
            Some(p) => format!("{p} needs the password of {user}"),
            None => format!("Password of {user}"),
        };
        let view = View { message: req.message.clone(), asker, ask: None, note: None };
        let prompt = match Prompt::new(view) {
            Ok(p) => p,
            Err(e) => return Err((req, format!("window: {e}"))),
        };
        let mut s = Session { req, user, helper: None, pam_error: false, prompt };
        if let Err(e) = s.attempt() {
            return Err((s.req, format!("helper: {e}")));
        }
        Ok(s)
    }

    /// A fresh helper run.
    fn attempt(&mut self) -> std::io::Result<()> {
        self.helper = Some(Helper::start(&self.user, &self.req.cookie)?);
        self.pam_error = false;
        self.prompt.view.ask = None;
        self.prompt.redraw();
        Ok(())
    }

    pub fn helper_fd(&self) -> Option<PollFd> {
        self.helper.as_ref().map(|h| PollFd::new(h.fd(), POLLIN))
    }

    /// The helper is readable.
    pub fn on_helper(&mut self) -> Option<Outcome> {
        let lines = match self.helper.as_mut()?.read() {
            Ok(l) => l,
            Err(e) => return Some(Err((FAILED, format!("helper: {e}")))),
        };
        let v = &mut self.prompt.view;
        for line in lines {
            match line {
                Line::Prompt { echo, text } => v.ask = Some((echo, text)),
                Line::Error(t) => (v.note, self.pam_error) = (Some((t, true)), true),
                Line::Info(t) => v.note = Some((t, false)),
                Line::Done(true) => return Some(Ok(())),
                Line::Done(false) => return self.retry(),
            }
        }
        self.prompt.redraw();
        None
    }

    fn retry(&mut self) -> Option<Outcome> {
        if !self.pam_error {
            self.prompt.view.note = Some(("Wrong password, try again".into(), true));
        }
        helper::wipe(&mut self.prompt.secret);
        match self.attempt() {
            Ok(()) => None,
            Err(e) => Some(Err((FAILED, format!("helper: {e}")))),
        }
    }

    /// The window was readable (or a timer is due), then draws.
    pub fn on_window(&mut self, readable: bool) -> Option<Outcome> {
        let act = match self.prompt.dispatch(readable) {
            Ok(a) => a,
            Err(e) => return Some(Err((CANCELLED, format!("window: {e}")))),
        };
        let act = match (act, self.prompt.draw()) {
            (_, Err(e)) => return Some(Err((CANCELLED, format!("window: {e}")))),
            (a, Ok(b)) => a.or(b),
        };
        match act? {
            Action::Cancel => Some(Err((CANCELLED, "dismissed".into()))),
            Action::Submit => self.submit(),
        }
    }

    fn submit(&mut self) -> Option<Outcome> {
        let h = self.helper.as_mut()?;
        self.prompt.view.ask = None;
        self.prompt.redraw();
        // Wipes the secret whether or not the write worked.
        match h.answer(&mut self.prompt.secret) {
            Ok(()) => None,
            Err(e) => Some(Err((FAILED, format!("helper: {e}")))),
        }
    }
}
