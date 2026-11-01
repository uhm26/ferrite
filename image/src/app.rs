//! The window loop: input in, a frame out when something changed. Nothing animates,
//! so idle sleeps in poll with no timeout (0 CPU). Loop from ../files/src/app.rs.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use frame::sys::{self, POLLIN, PollFd};
use frame::window::{Canvas, Event, Pointer, Rect, Window};
use frame::wl::Error;
use frame::wl::proto::pointer;
use frame::xkb::Key;

use crate::show::Show;
use crate::view::STEP;

/// Smooth-scroll px that count as one wheel click (frame's WHEEL_STEP).
const PX_PER_CLICK: f64 = frame::ui::theme::WHEEL_STEP as f64;

const USAGE: &str = "usage: image FILE|FOLDER  (headless: image --png OUT [--size WxH] FILE)";

struct App {
    win: Window,
    show: Show,
    pos: (f64, f64),
    drag: bool,
    quit: bool,
    titled: String,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--png") {
        return shot(&args);
    }
    let [path] = args.as_slice() else {
        return Err(USAGE.into());
    };
    let show = Show::open(&PathBuf::from(path))?;
    let mut win: Window = Window::new("image", "image", true)?;
    let (w, h) = win.configured_size();
    win.set_size(if w == 0 { 960 } else { w }, if h == 0 { 640 } else { h });
    let mut app = App {
        win,
        show,
        pos: (0.0, 0.0),
        drag: false,
        quit: false,
        titled: String::new(),
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
            if self.quit {
                return Ok(());
            }
            if dirty && !self.win.frame_pending() && self.draw()? {
                dirty = false;
            }
            self.win.flush()?;
            let timeout = self.win.next_deadline().map_or(-1, |at| {
                at.saturating_duration_since(Instant::now()).as_millis() as i32
            });
            sys::wait(&mut [PollFd::new(self.win.fd(), POLLIN)], timeout)?;
            self.win.dispatch()?;
            self.win.tick(Instant::now());
        }
    }

    /// One window event. True if it needs a redraw.
    fn on_event(&mut self, ev: Event) -> bool {
        let v = &mut self.show.view;
        match ev {
            Event::Close => self.quit = true,
            Event::Resize(w, h) if w > 0 && h > 0 => self.win.set_size(w, h),
            Event::Key(k) => return self.key(&k),
            Event::Pointer(Pointer::Motion(x, y)) => {
                if self.drag {
                    v.pan(x - self.pos.0, y - self.pos.1);
                }
                self.pos = (x, y);
                return self.drag;
            }
            Event::Pointer(Pointer::Button { button, pressed }) if button == pointer::BTN_LEFT => {
                self.drag = pressed;
                return false;
            }
            Event::Pointer(Pointer::Scroll { discrete, px }) => {
                let clicks = if discrete != 0 {
                    f64::from(discrete)
                } else {
                    px / PX_PER_CLICK
                };
                v.zoom_at(STEP.powf(-clicks), self.pos); // wheel down = zoom out
            }
            _ => return false,
        }
        true
    }

    /// Keys (keysyms, xkbcommon-keysyms.h). True if it needs a redraw.
    fn key(&mut self, k: &Key) -> bool {
        let mid = (self.win.width as f64 / 2.0, self.win.height as f64 / 2.0);
        let s = &mut self.show;
        match k.sym {
            0x71 | 0xff1b => self.quit = true,             // q, Escape
            0xff53 | 0x20 | 0xff56 => return s.step(1),    // Right, Space, Page_Down
            0xff51 | 0xff08 | 0xff55 => return s.step(-1), // Left, BackSpace, Page_Up
            0xff50 => return s.step(-isize::MAX),          // Home
            0xff57 => return s.step(isize::MAX),           // End
            0x2b | 0x3d | 0xffab => s.view.zoom_at(STEP, mid),  // + = KP_Add
            0x2d | 0xffad => s.view.zoom_at(1.0 / STEP, mid),   // - KP_Subtract
            0x30 | 0x66 => s.view.fit(),                        // 0, f
            0x31 => s.view.actual(mid),                         // 1
            _ => return false,
        }
        true
    }

    /// Draw a full frame into a free buffer. False if none is free yet.
    fn draw(&mut self) -> Result<bool, Error> {
        let Some(i) = self.win.acquire()? else {
            return Ok(false);
        };
        let (c, ()) = self.win.canvas(i);
        let full = Rect {
            x: 0,
            y: 0,
            w: c.w,
            h: c.h,
        };
        self.show.paint(c);
        self.win.present(i, &[full])?;
        let title = self.show.title();
        if title != self.titled {
            self.win.set_title(&title)?;
            self.titled = title;
        }
        Ok(true)
    }
}

/// Render one frame into a PNG, no compositor needed, for checking the look:
/// `image --png OUT [--size WxH] FILE`.
fn shot(args: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<&str> = args.iter().filter_map(|a| a.to_str()).collect();
    let val = |f: &str| {
        args.iter()
            .position(|a| *a == f)
            .and_then(|i| args.get(i + 1).copied())
    };
    let (w, h) = val("--size")
        .and_then(|s| s.split_once('x'))
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .unwrap_or((960, 640));
    let (Some(out), Some(file)) = (val("--png"), args.last()) else {
        return Err(USAGE.into());
    };
    let mut show = Show::open(&PathBuf::from(file))?;
    let mut px = vec![0u32; w * h];
    let t = Instant::now();
    show.paint(Canvas { px: &mut px, w, h });
    eprintln!("{} · painted in {:?}", show.title(), t.elapsed());
    frame::png::write(out, w, h, &px)?;
    Ok(())
}
