//! The window loop: input in, a frame out when something changed. Idle (no glide,
//! no animation) sleeps in poll with no timeout (0 CPU). Loop from ../files/src/app.rs.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use frame::sys::{self, POLLIN, PollFd};
use frame::window::{Canvas, Event, Pointer, Rect, Window};
use frame::wl::Error;
use frame::wl::proto::pointer;
use frame::xkb::Key;

use crate::show::Show;
use crate::shape::Shape;
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
    shape: Shape,
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
        shape: Shape::new((w, h)),
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
            dirty |= self.shape.turn(&mut self.win, self.show.size())?;
            if dirty && !self.win.frame_pending() && self.draw()? {
                dirty = self.show.view.moving(); // a glide draws on every frame callback
            }
            self.win.flush()?;
            // Wake for key repeat or the next animation frame, whichever is first.
            let next = match (self.win.next_deadline(), self.show.due()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let timeout = next.map_or(-1, |at| {
                at.saturating_duration_since(Instant::now()).as_millis() as i32
            });
            sys::wait(&mut [PollFd::new(self.win.fd(), POLLIN)], timeout)?;
            self.win.dispatch()?;
            let now = Instant::now();
            self.win.tick(now);
            dirty |= self.show.tick(now);
        }
    }

    /// One window event. True if it needs a redraw.
    fn on_event(&mut self, ev: Event) -> bool {
        let v = &mut self.show.view;
        match ev {
            Event::Close => self.quit = true,
            Event::Resize(w, h) if w > 0 && h > 0 => {
                self.shape.resized(w, h);
                self.win.set_size(w, h);
            }
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
        let by = match k.sym {
            0xff53 | 0x20 | 0xff56 => 1,    // Right, Space, Page_Down
            0xff51 | 0xff08 | 0xff55 => -1, // Left, BackSpace, Page_Up
            0xff50 => -isize::MAX,          // Home
            0xff57 => isize::MAX,           // End
            _ => 0,
        };
        let s = &mut self.show;
        let reshaped = match k.sym {
            _ if by != 0 => s.step(by),
            0x72 if !k.repeat => s.rotate(pix::CW),  // r
            0x52 if !k.repeat => s.rotate(pix::CCW), // R
            0xffff if !k.repeat => {
                s.trash(); // Delete
                true
            }
            _ => false,
        };
        if reshaped {
            return true;
        }
        match k.sym {
            0x63 if k.mods.ctrl && !k.repeat => self.copy(), // Ctrl+C
            0x71 | 0xff1b => self.quit = true,               // q, Escape
            0x2b | 0x3d | 0xffab => s.view.zoom_at(STEP, mid), // + = KP_Add
            0x2d | 0xffad => s.view.zoom_at(1.0 / STEP, mid), // - KP_Subtract
            0x30 | 0x66 => s.view.fit(mid),                  // 0, f
            0x31 => s.view.actual(mid),                      // 1
            _ => return false,
        }
        true
    }

    /// Put the frame on screen on the clipboard as a PNG. It stays pasteable while
    /// the viewer is open (no clipboard manager keeps it).
    fn copy(&mut self) {
        let done = self.show.png().and_then(|png| {
            let w = &mut self.win;
            w.copy_as(&["image/png"], png).map_err(|e| e.to_string())
        });
        self.show.note = match done {
            Ok(()) => "copied".into(),
            Err(e) => format!("can't copy: {e}"),
        };
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
        self.show.view.animate(Instant::now());
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
