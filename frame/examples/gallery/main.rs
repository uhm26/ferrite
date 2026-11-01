//! The gallery: every frame::ui component, the type, colors, icons and motion, laid
//! out like a frame app (sidebar, pages, groups), plus a debug overlay.
//! `cargo run --release --example gallery`
//! Headless: `… --example gallery -- --png out.png [--page N] [--size WxH] [--glass] [--debug]`.

mod debug;
mod layout;
mod pages;

use std::time::Instant;

use frame::sys::{self, POLLIN, PollFd};
use frame::ui::{State, Text, theme};
use frame::window::{Canvas, Event, Pointer, Rect, Window};
use frame::wl::Error;
use frame::wl::proto::{cursor_shape, pointer};

use debug::Stats;
use pages::Gallery;

/// Longest step a spring takes after a stall, s (no jumps after a hiccup).
const MAX_DT: f32 = 0.05;

struct App {
    win: Window,
    ui: State,
    gallery: Gallery,
    stats: Stats,
    last: Instant,
    glass: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let out = args.get(i + 1).map_or("gallery.png", String::as_str);
        return snapshot(out, &args);
    }
    let mut win: Window = Window::new("frame gallery", "frame-gallery", true)?;
    let (w, h) = win.configured_size();
    win.set_size(if w == 0 { 960 } else { w }, if h == 0 { 720 } else { h });
    let mut app = App {
        win,
        ui: State::new(Text::new(theme::FAMILY)?),
        gallery: Gallery::new(),
        stats: Stats::default(),
        last: Instant::now(),
        glass: false,
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
            if self.gallery.quit {
                return Ok(());
            }
            if (dirty || self.ui.animating()) && !self.win.frame_pending() && self.draw()? {
                dirty = false;
            }
            // Keys handled while drawing can ask to quit. Leave now: a hidden window gets no
            // frame callback, so wait could sleep until some unrelated event.
            if self.gallery.quit {
                return Ok(());
            }
            self.wait()?;
        }
    }

    /// Feed one window event to the ui. True if it needs a redraw.
    fn on_event(&mut self, ev: Event) -> bool {
        let i = &mut self.ui.input;
        match ev {
            Event::Close => self.gallery.quit = true,
            Event::Resize(w, h) if w > 0 && h > 0 => self.win.set_size(w, h),
            Event::Key(k) => i.keys.push(k),
            Event::Pointer(Pointer::Motion(x, y)) => i.pos = (x as f32, y as f32),
            Event::Pointer(Pointer::Button { button, pressed }) if button == pointer::BTN_LEFT => {
                i.button(pressed);
            }
            Event::Pointer(Pointer::Scroll { discrete, px }) => {
                self.gallery.scroll_by(discrete, px)
            }
            _ => return false,
        }
        true
    }

    /// Sleep until the compositor sends something (a frame callback while animating).
    /// Idle: no timeout at all, so 0 CPU.
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
        self.gallery.draw(&mut ui, &self.stats);
        ui.end();
        self.stats.frame(now, now.elapsed());
        self.stats.animating = self.ui.animating();
        self.win.present(i, &[full])?;
        let shape = if self.ui.pointer {
            cursor_shape::SHAPE_POINTER
        } else {
            cursor_shape::SHAPE_DEFAULT
        };
        self.win.set_cursor(shape)?;
        if self.ui.glass != self.glass {
            self.glass = self.ui.glass;
            self.win.set_opaque(!self.glass)?; // takes effect with the next commit
        }
        Ok(true)
    }
}

/// Render settled frames into a PNG (no compositor needed).
fn snapshot(out: &str, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let has = |f: &str| args.iter().any(|a| a == f);
    let arg = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
    };
    let (w, h) = arg("--size")
        .and_then(|s| s.split_once('x'))
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .unwrap_or((960, 640));
    let mut px = vec![0u32; w * h];
    let mut ui = State::new(Text::new(theme::FAMILY)?);
    ui.glass = has("--glass");
    let mut g = Gallery::new();
    g.debug = has("--debug");
    g.page = arg("--page").and_then(|p| p.parse().ok()).unwrap_or(0);
    ui.input.pos = (-1.0, -1.0);
    let mut stats = Stats::default();
    let start = Instant::now();
    for n in 0..40 {
        if n == 1 {
            // frame 0 rasterizes every glyph once; time the steady state after it
            stats = Stats::default();
        }
        let t = Instant::now();
        let mut f = ui.begin(Canvas { px: &mut px, w, h }, MAX_DT);
        g.draw(&mut f, &stats);
        f.end();
        stats.frame(Instant::now(), t.elapsed());
    }
    eprintln!(
        "{} frames in {:.1} ms; {}",
        40,
        start.elapsed().as_secs_f64() * 1e3,
        stats.summary()
    );
    // Glass shows the wallpaper through: preview it over white, the worst case.
    if ui.glass {
        px.iter_mut()
            .for_each(|p| *p = frame::ui::paint::over(0xffffffff, *p, 255));
    }
    frame::png::write(out, w, h, &px)?;
    Ok(())
}
