//! The window loop: events in, a frame out when something changed or moves.
//! Idle sleeps in poll with no timeout (0 CPU). Loop from frame's gallery.

use std::path::PathBuf;
use std::time::Instant;

use frame::sys::{self, POLLIN, PollFd};
use frame::ui::{State, Text, theme};
use frame::window::{Event, Pointer, Rect, Window};
use frame::wl::Error;
use frame::wl::proto::{cursor_shape, pointer};

use crate::browse::Browser;
use crate::view::View;

/// Longest step a spring takes after a stall, s (no jumps after a hiccup).
const MAX_DT: f32 = 0.05;

struct App {
    win: Window,
    ui: State,
    view: View,
    browser: Browser,
    last: Instant,
}

/// Open the window on the folder given as the first argument, else the current one.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let start = match std::env::args_os().nth(1) {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?,
    };
    let mut win: Window = Window::new("files", "files", true)?;
    let (w, h) = win.configured_size();
    win.set_size(if w == 0 { 720 } else { w }, if h == 0 { 560 } else { h });
    let mut app = App {
        win,
        ui: State::new(Text::new(theme::FAMILY)?),
        view: View::default(),
        browser: Browser::new(std::fs::canonicalize(&start)?),
        last: Instant::now(),
    };
    app.run()?;
    Ok(())
}

impl App {
    fn run(&mut self) -> Result<(), Error> {
        let mut dirty = true;
        loop {
            while let Some(ev) = self.win.next_event() {
                dirty |= self.on_event(ev);
            }
            if self.view.quit {
                return Ok(());
            }
            if (dirty || self.ui.animating()) && !self.win.frame_pending() && self.draw()? {
                dirty = false;
            }
            self.wait()?;
        }
    }

    /// Feed one window event to the ui. True if it needs a redraw.
    fn on_event(&mut self, ev: Event) -> bool {
        let i = &mut self.ui.input;
        match ev {
            Event::Close => self.view.quit = true,
            Event::Resize(w, h) if w > 0 && h > 0 => self.win.set_size(w, h),
            Event::Key(k) => i.keys.push(k),
            Event::Pointer(Pointer::Motion(x, y)) => i.pos = (x as f32, y as f32),
            Event::Pointer(Pointer::Button { button, pressed }) if button == pointer::BTN_LEFT => {
                i.button(pressed);
            }
            Event::Pointer(Pointer::Scroll { discrete, px }) => self.view.scroll_by(discrete, px),
            _ => return false,
        }
        true
    }

    /// Sleep until the compositor sends something (a frame callback while animating).
    fn wait(&mut self) -> Result<(), Error> {
        self.win.flush()?;
        let timeout = self.win.next_deadline().map_or(-1, |at| {
            at.saturating_duration_since(Instant::now()).as_millis() as i32
        });
        let mut fds = [PollFd::new(self.win.fd(), POLLIN)];
        sys::wait(&mut fds, timeout)?;
        self.win.dispatch()?;
        self.win.tick(Instant::now());
        Ok(())
    }

    /// Draw a full frame into a free buffer. False if none is free yet.
    fn draw(&mut self) -> Result<bool, Error> {
        let Some(i) = self.win.acquire()? else {
            return Ok(false);
        };
        let now = Instant::now();
        // After idle, start at dt 0 so a fresh animation doesn't jump.
        let dt = if self.ui.animating() {
            (now - self.last).as_secs_f32().min(MAX_DT)
        } else {
            0.0
        };
        self.last = now;
        let (c, ()) = self.win.canvas(i);
        let full = Rect {
            x: 0,
            y: 0,
            w: c.w,
            h: c.h,
        };
        let mut ui = self.ui.begin(c, dt);
        self.view.draw(&mut ui, &mut self.browser);
        ui.end();
        self.win.present(i, &[full])?;
        let shape = if self.ui.pointer {
            cursor_shape::SHAPE_POINTER
        } else {
            cursor_shape::SHAPE_DEFAULT
        };
        self.win.set_cursor(shape)?;
        Ok(true)
    }
}
