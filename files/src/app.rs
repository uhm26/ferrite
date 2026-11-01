//! The window loop: events in, a frame out when something changed or moves.
//! Idle sleeps in poll with no timeout (0 CPU). Loop from frame's gallery.

use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::Instant;

use frame::sys::watch::Watch;
use frame::sys::{self, POLLHUP, POLLIN, PollFd};
use frame::ui::{State, Text, theme};
use frame::window::{Canvas, Event, Pointer, Rect, Window};
use frame::wl::Error;
use frame::wl::proto::{cursor_shape, pointer};

use crate::browse::Browser;
use crate::view::View;

/// Longest step a spring takes after a stall, s (no jumps after a hiccup).
const MAX_DT: f32 = 0.05;
/// linux/input-event-codes.h: the mouse side buttons.
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

struct App {
    win: Window,
    ui: State,
    view: View,
    browser: Browser,
    last: Instant,
    /// The folder the window title names.
    titled: PathBuf,
    /// Live refresh: inotify on the folder in `watching`.
    watch: Option<Watch>,
    watching: PathBuf,
    /// A paste being read: the pipe from the clipboard owner and what came so far.
    paste: Option<(File, Vec<u8>)>,
}

/// Open the window on the folder given as the first argument, else home (like
/// Nemo; `files .` for here). `--png` renders headless instead (see [`shot`]).
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--png") {
        return shot(&args);
    }
    let start = args.first().map_or_else(crate::view::home, PathBuf::from);
    let mut win: Window = Window::new("files", "files", true)?;
    let (w, h) = win.configured_size();
    win.set_size(if w == 0 { 720 } else { w }, if h == 0 { 560 } else { h });
    let mut app = App {
        win,
        ui: State::new(Text::new(theme::FAMILY)?),
        view: View::new()?,
        // A folder we can't resolve still opens: Browser falls back and shows why.
        browser: Browser::new(std::fs::canonicalize(&start).unwrap_or(start)),
        last: Instant::now(),
        titled: PathBuf::new(),
        watch: None,
        watching: PathBuf::new(),
        paste: None,
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
            // Keys handled while drawing can ask to quit. Leave now: a hidden window gets no
            // frame callback, so wait could sleep until some unrelated event.
            if self.view.quit {
                return Ok(());
            }
            dirty |= self.wait()?;
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
            Event::Pointer(Pointer::Button {
                button,
                pressed: true,
            }) if button == pointer::BTN_RIGHT => {
                let pos = i.pos;
                self.view.right_click(pos);
            }
            Event::Pointer(Pointer::Scroll { discrete, px }) => self.view.scroll_by(discrete, px),
            // Mouse side buttons: back / forward, like Nemo.
            Event::Pointer(Pointer::Button {
                button: BTN_SIDE,
                pressed: true,
            }) => self.browser.travel(true),
            Event::Pointer(Pointer::Button {
                button: BTN_EXTRA,
                pressed: true,
            }) => self.browser.travel(false),
            _ => return false,
        }
        true
    }

    /// Sleep until the compositor sends something (a frame callback while animating).
    /// Also wakes when the shown folder changes on disk; then it is read again
    /// and this returns true (needs a redraw).
    fn wait(&mut self) -> Result<bool, Error> {
        self.win.flush()?;
        if self.watching != self.browser.cwd {
            self.watching.clone_from(&self.browser.cwd);
            // shortcut: no live refresh where inotify fails (some network mounts).
            self.watch = Watch::new(&[&self.watching]).ok();
        }
        let timeout = self.win.next_deadline().map_or(-1, |at| {
            at.saturating_duration_since(Instant::now()).as_millis() as i32
        });
        let mut fds = vec![PollFd::new(self.win.fd(), POLLIN)];
        // Optional fds after the window: the folder watch, then a paste in flight.
        let watch_at = self.watch.as_ref().map(|w| {
            fds.push(PollFd::new(w.fd(), POLLIN));
            fds.len() - 1
        });
        let paste_at = self.paste.as_ref().map(|(f, _)| {
            fds.push(PollFd::new(f.as_fd(), POLLIN | POLLHUP));
            fds.len() - 1
        });
        fds.push(PollFd::new(self.view.jobs_fd(), POLLIN));
        let jobs_at = fds.len() - 1;
        sys::wait(&mut fds, timeout)?;
        let ready = |at: Option<usize>| at.is_some_and(|i| fds[i].ready(POLLIN | POLLHUP));
        let (changed, pasting) = (ready(watch_at), ready(paste_at));
        let jobs_done = fds[jobs_at].ready(POLLIN);
        drop(fds);
        self.win.dispatch()?;
        self.win.tick(Instant::now());
        let pasted = pasting && self.read_paste();
        if jobs_done {
            self.view.job_reports(&mut self.browser);
        }
        if changed && self.watch.as_mut().is_some_and(Watch::drain) {
            self.browser.reload();
            // Re-arm: if the folder was removed and made again, the old watch is dead.
            self.watching.clear();
            return Ok(true);
        }
        Ok(pasted || jobs_done)
    }

    /// Read what the clipboard owner wrote into the paste pipe. At the end (EOF or an
    /// error) hand the text to the view; true then (needs a redraw).
    fn read_paste(&mut self) -> bool {
        let Some((file, buf)) = &mut self.paste else {
            return false;
        };
        let mut chunk = [0u8; 4096];
        loop {
            match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return false,
                Err(_) => break,
            }
        }
        let text = self
            .paste
            .take()
            .map(|(_, b)| String::from_utf8_lossy(&b).into_owned());
        self.view.paste(text.unwrap_or_default());
        true
    }

    /// After a frame: copy what the view wants copied, start reading a paste.
    fn clipboard(&mut self) -> Result<(), Error> {
        let (want, copy) = self.view.clipboard();
        if let Some(t) = copy {
            self.win.copy(t)?;
        }
        if want {
            // shortcut: text only; an image on the clipboard pastes nothing.
            self.paste = self
                .win
                .paste(false)?
                .map(|(fd, _)| (File::from(fd), Vec::new()));
        }
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
        let shape = if self.view.resize_cursor() {
            cursor_shape::SHAPE_EW_RESIZE
        } else if self.ui.text_cursor {
            cursor_shape::SHAPE_TEXT
        } else if self.ui.pointer {
            cursor_shape::SHAPE_POINTER
        } else {
            cursor_shape::SHAPE_DEFAULT
        };
        self.win.set_cursor(shape)?;
        self.clipboard()?;
        if self.titled != self.browser.cwd {
            self.titled.clone_from(&self.browser.cwd);
            let name = self
                .titled
                .file_name()
                .map_or("/".into(), |n| n.to_string_lossy());
            self.win.set_title(&name)?;
        }
        Ok(true)
    }
}

/// Render settled frames into a PNG, no compositor needed, for checking the look:
/// `files --png out.png [--size WxH] [--grid] [--select N] [--pointer X,Y] [--menu] [folder]`
/// (`--menu` right-clicks at the pointer).
fn shot(args: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<&str> = args.iter().filter_map(|a| a.to_str()).collect();
    let val = |f: &str| {
        args.iter()
            .position(|a| *a == f)
            .and_then(|i| args.get(i + 1).copied())
    };
    let pair = |f: &str, sep: char| -> Option<(f32, f32)> {
        let (a, b) = val(f)?.split_once(sep)?;
        Some((a.parse().ok()?, b.parse().ok()?))
    };
    let (w, h) = pair("--size", 'x').unwrap_or((960.0, 600.0));
    let (w, h) = (w as usize, h as usize);
    // The folder is the one argument that is neither a flag nor a flag's value.
    let takes_value = ["--png", "--size", "--select", "--pointer"];
    let folder = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || !takes_value.contains(&args[i - 1])))
        .map(|(_, a)| PathBuf::from(a))
        .next_back()
        .unwrap_or_else(crate::view::home);
    let mut browser = Browser::new(std::fs::canonicalize(&folder).unwrap_or(folder));
    browser.sel = val("--select").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut view = View::new()?;
    view.set_grid(args.contains(&"--grid"));
    let mut ui = State::new(Text::new(theme::FAMILY)?);
    ui.input.pos = pair("--pointer", ',').unwrap_or((-1.0, -1.0));
    if args.contains(&"--menu") {
        view.right_click(ui.input.pos);
    }
    let mut px = vec![0u32; w * h];
    for _ in 0..40 {
        let mut f = ui.begin(Canvas { px: &mut px, w, h }, MAX_DT);
        view.draw(&mut f, &mut browser);
        f.end();
    }
    frame::png::write(val("--png").unwrap_or("files.png"), w, h, &px)?;
    Ok(())
}
