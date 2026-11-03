//! The popup: what is asked, by whom, a password field and two buttons. One window per
//! request; dropping the `Prompt` closes it. Keys go to our own field so the password
//! lives in one fixed buffer that is wiped, never in a growing `String`.

use frame::sys::{POLLIN, PollFd};
use frame::ui::text::{Ink, MEDIUM, REGULAR, SEMIBOLD};
use frame::ui::theme::*;
use frame::ui::{Align, Rect, State, Text, Ui};
use frame::window::{self, Event, Pointer, Window};
use frame::wl::Error;
use frame::wl::proto::{cursor_shape, pointer};
use std::time::Instant;

const W: f32 = 400.0;
const LINE_H: f32 = 20.0;
const MAX_LINES: usize = 4;
/// Longest password we take; the buffer never grows past it (+1 for the helper's `\n`).
const MAX_SECRET: usize = 512;
const MAX_DT: f32 = 0.05;
// xkbcommon-keysyms.h
const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
const KEY_KP_ENTER: u32 = 0xff8d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_U: u32 = 0x75;

/// What the user did.
pub enum Action {
    Submit,
    Cancel,
}

/// What the window shows besides the field.
pub struct View {
    pub message: String,
    /// "files needs the password of gabe".
    pub asker: String,
    /// The helper's prompt ("Password: "), `None` while it isn't asking.
    pub ask: Option<(bool, String)>,
    /// Last note from PAM or us; true = error.
    pub note: Option<(String, bool)>,
}

pub struct Prompt {
    win: Window,
    ui: State,
    pub view: View,
    /// What is typed. Capacity fixed at creation: pushes never reallocate.
    pub secret: Vec<u8>,
    lines: Vec<String>,
    last: Instant,
    dirty: bool,
}

impl Prompt {
    pub fn new(view: View) -> Result<Prompt, Box<dyn std::error::Error>> {
        let mut text = Text::new(FAMILY)?;
        let mut lines = wrap(&mut text, &view.message, W - 2.0 * S24, body());
        lines.truncate(MAX_LINES);
        let mut win: Window = Window::new("Authentication", "auth", true)?;
        let h = height(lines.len()) as usize;
        win.set_size_limits((W as usize, h), (W as usize, h))?;
        win.set_size(W as usize, h);
        Ok(Prompt {
            win,
            ui: State::new(text),
            view,
            secret: Vec::with_capacity(MAX_SECRET + 1),
            lines,
            last: Instant::now(),
            dirty: true,
        })
    }

    pub fn poll_fd(&self) -> PollFd {
        PollFd::new(self.win.fd(), POLLIN)
    }

    /// Ms until key repeat is due, -1 for none.
    pub fn timeout(&self) -> i32 {
        self.win.next_deadline().map_or(-1, |at| {
            at.saturating_duration_since(Instant::now()).as_millis() as i32
        })
    }

    pub fn redraw(&mut self) {
        self.dirty = true;
    }

    /// Handles what the compositor sent (call after poll, readable or not: key repeat).
    pub fn dispatch(&mut self, readable: bool) -> Result<Option<Action>, Error> {
        if readable {
            self.win.dispatch()?;
        }
        self.win.tick(Instant::now());
        let mut act = None;
        while let Some(ev) = self.win.next_event() {
            act = act.or(self.on_event(ev));
            self.dirty = true;
        }
        Ok(act)
    }

    fn on_event(&mut self, ev: Event) -> Option<Action> {
        let i = &mut self.ui.input;
        match ev {
            Event::Close => return Some(Action::Cancel),
            Event::Resize(w, h) if w > 0 && h > 0 => self.win.set_size(w, h),
            Event::Key(k) => return self.on_key(&k),
            Event::Pointer(Pointer::Motion(x, y)) => i.pos = (x as f32, y as f32),
            Event::Pointer(Pointer::Button { button, pressed }) if button == pointer::BTN_LEFT => {
                i.button(pressed);
            }
            _ => {}
        }
        None
    }

    fn on_key(&mut self, k: &frame::xkb::Key) -> Option<Action> {
        match k.sym {
            KEY_ESCAPE => return Some(Action::Cancel),
            KEY_RETURN | KEY_KP_ENTER if self.view.ask.is_some() => return Some(Action::Submit),
            _ if self.view.ask.is_none() => {}
            KEY_BACKSPACE if k.mods.ctrl => crate::helper::wipe(&mut self.secret),
            KEY_U if k.mods.ctrl => crate::helper::wipe(&mut self.secret),
            KEY_BACKSPACE => {
                // Drop one whole char: continuation bytes are 10xxxxxx.
                while let Some(b) = self.secret.pop() {
                    if b & 0xc0 != 0x80 {
                        break;
                    }
                }
            }
            _ if k.mods.ctrl || k.mods.alt || k.mods.logo => {}
            _ => {
                let t = k.text.as_bytes();
                let printable = !t.is_empty() && !k.text.chars().any(char::is_control);
                if printable && self.secret.len() + t.len() <= MAX_SECRET {
                    self.secret.extend_from_slice(t);
                }
            }
        }
        None
    }

    /// Draws if something changed and the compositor is ready. Buttons act through `Action`.
    pub fn draw(&mut self) -> Result<Option<Action>, Error> {
        if !(self.dirty || self.ui.animating()) || self.win.frame_pending() {
            return Ok(None);
        }
        let Some(i) = self.win.acquire()? else {
            return Ok(None);
        };
        let now = Instant::now();
        let dt = if self.ui.animating() { (now - self.last).as_secs_f32().min(MAX_DT) } else { 0.0 };
        self.last = now;
        let (c, ()) = self.win.canvas(i);
        let full = window::Rect { x: 0, y: 0, w: c.w, h: c.h };
        let mut ui = self.ui.begin(c, dt);
        let act = layout(&mut ui, &self.view, &self.lines, &self.secret);
        ui.end();
        self.dirty = false;
        self.win.present(i, &[full])?;
        let shape = if self.ui.pointer { cursor_shape::SHAPE_POINTER } else { cursor_shape::SHAPE_DEFAULT };
        self.win.set_cursor(shape)?;
        self.win.flush()?;
        Ok(act)
    }

    pub fn flush(&mut self) -> Result<(), Error> {
        self.win.flush()
    }
}

impl Drop for Prompt {
    fn drop(&mut self) {
        crate::helper::wipe(&mut self.secret);
    }
}

fn body() -> Ink {
    Ink::new(BODY, REGULAR, TEXT)
}

/// Window height for `n` message lines: title, message, asker, field, note, buttons.
fn height(n: usize) -> f32 {
    S24 + 28.0 + S8 + n as f32 * LINE_H + S8 + 16.0 + S16 + CONTROL_H + S8 + 16.0 + S16 + CONTROL_H + S24
}

/// Greedy word wrap to `max` px.
fn wrap(text: &mut Text, s: &str, max: f32, ink: Ink) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && text.width(&next, ink) > max {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        } else {
            cur = next;
        }
    }
    lines.push(cur);
    lines
}

fn layout(ui: &mut Ui, v: &View, lines: &[String], secret: &[u8]) -> Option<Action> {
    let pal = ui.pal;
    ui.p.clear(pal.window);
    let (x, w) = (S24, W - 2.0 * S24);
    let mut y = S24;
    let title = Ink::new(TITLE, SEMIBOLD, pal.text);
    ui.text(Rect::new(x, y, w, 28.0), "Authentication required", title, Align::Left);
    y += 28.0 + S8;
    for l in lines {
        ui.text(Rect::new(x, y, w, LINE_H), l, body().color(pal.text), Align::Left);
        y += LINE_H;
    }
    y += S8;
    let small = Ink::new(SMALL, MEDIUM, pal.sub);
    ui.text(Rect::new(x, y, w, 16.0), &v.asker, small, Align::Left);
    y += 16.0 + S16;
    field(ui, Rect::new(x, y, w, CONTROL_H), v, secret);
    y += CONTROL_H + S8;
    if let Some((note, err)) = &v.note {
        let ink = small.color(if *err { ERROR } else { pal.sub });
        ui.text(Rect::new(x, y, w, 16.0), note, ink, Align::Left);
    }
    y += 16.0 + S16;
    let (bw, gap) = (112.0, S8);
    let ok = Rect::new(x + w - bw, y, bw, CONTROL_H);
    let cancel = Rect::new(ok.x - gap - bw, y, bw, CONTROL_H);
    if ui.button("cancel", cancel, "Cancel", false) {
        return Some(Action::Cancel);
    }
    let go = ui.button("ok", ok, "Authenticate", true);
    (go && v.ask.is_some()).then_some(Action::Submit)
}

/// The password field: always focused, dots (or the text for an echo prompt), still caret.
fn field(ui: &mut Ui, r: Rect, v: &View, secret: &[u8]) {
    let pal = ui.pal;
    ui.p.rect(r, RADIUS, pal.hover);
    ui.p.stroke(r, RADIUS, 1.0, pal.sub);
    let ink = body().color(pal.text);
    let y = ui.s.text.center(r.y, r.h, ink);
    let x0 = r.x + S12;
    let Some((echo, ask)) = &v.ask else {
        ui.label((x0, y), "Checking…", ink.color(pal.sub));
        return;
    };
    let shown = if *echo {
        String::from_utf8_lossy(secret).into_owned()
    } else {
        "•".repeat(String::from_utf8_lossy(secret).chars().count())
    };
    ui.p.clip(Some(r.inset(1.0)));
    if shown.is_empty() {
        let hint = ask.trim().trim_end_matches(':');
        ui.label((x0, y), hint, ink.color(pal.sub));
    }
    // Long input scrolls left so the caret stays in view.
    let tw = ui.s.text.width(&shown, ink);
    let x = x0 - (tw - (r.w - 2.0 * S12)).max(0.0);
    let end = ui.label((x, y), &shown, ink);
    let cap = ui.s.text.metrics(ink.px, ink.style).cap;
    ui.p.rect(Rect::new(end.round(), y - cap - 2.0, 2.0, cap + 4.0), 0.0, ACCENT);
    ui.p.clip(None);
}
