//! The loop: sleep in poll on the Wayland socket and mpv's wake pipe; on wake,
//! take mpv's news, draw a video frame if one is due (GPU only), and redraw the
//! controls only when what they show changed. Playing with the bar hidden costs
//! no CPU drawing at all. Loop shape from ../image/src/app.rs.

use std::time::{Duration, Instant};

use frame::sys::{self, POLLIN, PollFd};
use frame::ui::State as UiState;
use frame::ui::Text;
use frame::ui::theme::{FAMILY, WHEEL_STEP};
use frame::window::{Event, Pointer, Rect, Window};
use frame::wl::proto::{cursor_shape, pointer};
use frame::xkb::Key;

use crate::controls::{self, Action, Controls, Look};
use crate::gpu::Gpu;
use crate::player::Player;
use crate::video::Video;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// The bar hides this long after the pointer last moved (while playing).
const HIDE_AFTER: Duration = Duration::from_millis(2000);
/// Two clicks on the video closer than this: fullscreen.
const DOUBLE_CLICK: Duration = Duration::from_millis(300);
const MAX_DT: f32 = 0.05;
const USAGE: &str =
    "usage: video FILE  (headless look check: video --png OUT [--size WxH] [--pointer X,Y])";

/// What a window buffer holds outside the bar zone, so the next draw into it
/// knows whether the whole buffer must be painted again.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Painted {
    w: usize,
    h: usize,
    /// 0 video shows through, 1 black (no frame yet), 2 failed message.
    mode: u8,
}

// Field order is drop order: mpv and the GPU buffers go while the GL context lives.
struct App {
    player: Player,
    video: Video,
    gpu: Gpu,
    win: Window<Painted>,
    ui: UiState,
    controls: Controls,
    /// The bar stays until then (pointer motion, keys).
    awake_until: Instant,
    visible: bool,
    last_draw: Instant,
    /// The last click on the picture, and whether a double-click undoes its toggle.
    last_click: Option<(Instant, bool)>,
    frame_due: bool,
    titled: String,
    quit: bool,
    /// What the bar showed at the last state change (see `look_key`).
    drawn_key: [i64; 8],
}

pub fn run() -> Res<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--png") {
        return crate::shot::run(&args);
    }
    let [file] = args.as_slice() else {
        return Err(USAGE.into());
    };
    let gpu = Gpu::open()?;
    let mut win: Window<Painted> = Window::new("video", "video", false)?;
    let (w, h) = win.configured_size();
    win.set_size(if w == 0 { 1280 } else { w }, if h == 0 { 720 } else { h });
    let mut video = Video::new(&mut win)?;
    let (vw, vh) = (win.width as u32, win.height as u32);
    video.resize(vw, vh);
    let player = Player::new(file, &gpu)?;
    let now = Instant::now();
    let mut app = App {
        player,
        video,
        gpu,
        win,
        ui: UiState::new(Text::new(FAMILY)?),
        controls: Controls::default(),
        awake_until: now + HIDE_AFTER,
        visible: true,
        last_draw: now,
        last_click: None,
        frame_due: false,
        titled: String::new(),
        quit: false,
        drawn_key: [0; 8],
    };
    app.ui.input.pos = (-1.0, -1.0); // outside until the pointer enters
    app.run()
}

impl App {
    fn run(&mut self) -> Res<()> {
        let mut dirty = true;
        loop {
            if self.player.poll() {
                // time-pos changes every video frame; the bar only every second or px.
                let key = self.look_key();
                dirty |= key != self.drawn_key;
                self.drawn_key = key;
                self.retitle()?;
            }
            while let Some(ev) = self.win.next_foreign() {
                if self.video.on_event(&mut self.win, &ev)? {
                    self.player.report_swap();
                }
            }
            while let Some(ev) = self.win.next_event() {
                dirty |= self.on_event(ev)?;
            }
            if self.quit || self.player.quit {
                return Ok(());
            }
            self.frame_due |= self.player.frame_due();
            if self.frame_due && self.render()? {
                self.frame_due = false;
            }
            let now = Instant::now();
            dirty |= self.update_visible(now);
            let busy = dirty || self.ui.animating();
            if busy && !self.win.frame_pending() && self.draw()? {
                dirty = false;
            }
            self.win.flush()?;
            self.sleep(now)?;
        }
    }

    /// Wait for the compositor, mpv, key repeat or the bar's hide time.
    fn sleep(&mut self, now: Instant) -> Res<()> {
        let hide = (self.visible && !self.held_open()).then_some(self.awake_until);
        let next = [self.win.next_deadline(), hide].into_iter().flatten().min();
        let timeout = next.map_or(-1, |at| {
            at.saturating_duration_since(now).as_millis() as i32 + 1
        });
        let mut fds = [
            PollFd::new(self.win.fd(), POLLIN),
            PollFd::new(self.player.fd(), POLLIN),
        ];
        sys::wait(&mut fds, timeout)?;
        self.win.dispatch()?;
        self.win.tick(Instant::now());
        Ok(())
    }

    /// Draw mpv's frame into a free GPU buffer and show it. False if none is free.
    fn render(&mut self) -> Res<bool> {
        let Some(i) = self.video.acquire(&mut self.win, &self.gpu)? else {
            return Ok(false);
        };
        let was_shown = self.video.shown;
        self.player.render(self.video.target(i))?;
        self.gpu.flush();
        self.video.present(&mut self.win, i)?;
        if !was_shown {
            self.win.flush()?; // the window turns see-through on its next draw
        }
        Ok(true)
    }

    /// The bar must stay: nothing plays, or the owner is using it.
    fn held_open(&self) -> bool {
        let s = &self.player.state;
        s.paused
            || s.ended
            || !self.video.shown
            || self.player.failed.is_some()
            || self.controls.bar.contains(self.ui.input.pos)
            || self.ui.input.down
    }

    /// Show or hide the bar (and the cursor with it). True if that changed.
    fn update_visible(&mut self, now: Instant) -> bool {
        let v = now < self.awake_until || self.held_open();
        let changed = v != self.visible;
        self.visible = v;
        changed
    }

    fn wake(&mut self) {
        self.awake_until = Instant::now() + HIDE_AFTER;
    }

    /// One window event. True if it needs a redraw.
    fn on_event(&mut self, ev: Event) -> Res<bool> {
        match ev {
            Event::Close => self.quit = true,
            Event::Resize(w, h) if w > 0 && h > 0 => {
                self.win.set_size(w, h);
                self.video.resize(w as u32, h as u32);
                self.frame_due = true; // redraw the paused frame at the new size
            }
            Event::Key(k) => {
                self.key(&k)?;
                self.wake();
            }
            Event::Mods(m) => self.ui.input.mods = m,
            Event::Pointer(p) => return self.pointer(p),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// One pointer event. True if the bar must redraw: motion over the picture
    /// changes nothing there (showing a hidden bar is `update_visible`'s job).
    fn pointer(&mut self, p: Pointer) -> Res<bool> {
        let zone = controls::zone_top(self.win.height) as f32;
        match p {
            Pointer::Motion(x, y) => {
                let was = self.ui.input.pos.1;
                self.ui.input.pos = (x as f32, y as f32);
                self.wake();
                // Entering (was outside) needs a draw too: it sets the cursor.
                let near = |y: f32| y < 0.0 || y >= zone;
                return Ok(near(was) || near(y as f32) || self.ui.input.down);
            }
            Pointer::Leave => self.ui.input.pos = (-1.0, -1.0), // the bar may hide now
            Pointer::Button { button, pressed } if button == pointer::BTN_LEFT => {
                self.ui.input.button(pressed);
                let on_bar = self.controls.bar.contains(self.ui.input.pos);
                if pressed && on_bar {
                    self.last_click = None; // a bar click breaks a double-click
                } else if pressed {
                    self.click_video()?;
                }
            }
            Pointer::Scroll { discrete, px } => {
                let clicks = if discrete != 0 {
                    f64::from(discrete)
                } else {
                    px / f64::from(WHEEL_STEP)
                };
                self.act(Action::Louder(-5.0 * clicks))?; // wheel down = quieter
                self.wake();
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// A click on the picture pauses; a second one soon after goes fullscreen
    /// (and the two pauses cancel out; a replay from the end stays).
    fn click_video(&mut self) -> Res<()> {
        let now = Instant::now();
        match self.last_click.take() {
            Some((t, undo)) if now - t < DOUBLE_CLICK => {
                if undo {
                    self.act(Action::Toggle)?;
                }
                self.act(Action::Fullscreen)
            }
            _ => {
                self.last_click = Some((now, !self.player.state.ended));
                self.act(Action::Toggle)
            }
        }
    }

    /// Keys (keysyms, xkbcommon-keysyms.h).
    fn key(&mut self, k: &Key) -> Res<()> {
        let sym = match k.sym {
            0x41..=0x5a => k.sym + 0x20, // A-Z: Caps Lock or Shift, same key
            s => s,
        };
        let act = match sym {
            // Holding a key repeats only seeking and volume (not pause, mute, fullscreen).
            _ if k.repeat && !matches!(sym, 0xff51..=0xff54 | 0x6a | 0x6c) => return Ok(()),
            0x20 | 0x6b => Action::Toggle,     // Space, k
            0xff51 => Action::Skip(-5.0),      // Left
            0xff53 => Action::Skip(5.0),       // Right
            0x6a => Action::Skip(-10.0),       // j
            0x6c => Action::Skip(10.0),        // l
            0xff50 => Action::Seek(0.0, true), // Home
            0xff52 => Action::Louder(5.0),     // Up
            0xff54 => Action::Louder(-5.0),    // Down
            0x6d => Action::Mute,              // m
            0x66 => Action::Fullscreen,        // f
            0xff1b if self.win.fullscreen => Action::Fullscreen, // Escape leaves it
            0xff1b | 0x71 => {
                self.quit = true; // Escape, q
                return Ok(());
            }
            _ => return Ok(()),
        };
        self.act(act)
    }

    fn act(&mut self, a: Action) -> Res<()> {
        let p = &mut self.player;
        let done = match a {
            Action::Toggle if p.state.ended => p
                .command(&["seek", "0", "absolute"])
                .and(p.command(&["set", "pause", "no"])),
            Action::Toggle => p.command(&["cycle", "pause"]),
            Action::Seek(t, exact) => {
                let t = t.clamp(0.0, p.state.duration.max(0.0)).to_string();
                p.command(&[
                    "seek",
                    &t,
                    if exact {
                        "absolute+exact"
                    } else {
                        "absolute+keyframes"
                    },
                ])
            }
            // Relative: mpv lands on a keyframe in that direction, and presses made
            // before it reports the new time add up (absolute+keyframes can land behind).
            Action::Skip(d) => p.command(&["seek", &d.to_string(), "relative"]),
            Action::Volume(v) => p.set_double("volume", v.clamp(0.0, 100.0)),
            Action::Louder(d) => p.command(&["add", "volume", &d.to_string()]),
            Action::Mute => p.command(&["cycle", "mute"]),
            Action::Fullscreen => {
                let on = !self.win.fullscreen;
                return Ok(self.win.set_fullscreen(on)?);
            }
        };
        if let Err(e) = done {
            eprintln!("video: {e}"); // e.g. seeking before the file loaded: not fatal
        }
        Ok(())
    }

    /// Redraw the bar zone (or the whole window when its content changed) into a
    /// free buffer. False if none is free yet.
    fn draw(&mut self) -> Res<bool> {
        let Some(i) = self.win.acquire()? else {
            return Ok(false);
        };
        let now = Instant::now();
        let dt = if self.ui.animating() {
            (now - self.last_draw).as_secs_f32().min(MAX_DT)
        } else {
            0.0
        };
        self.last_draw = now;
        let failed = self.player.failed.as_deref();
        let mode = if failed.is_some() {
            2
        } else {
            u8::from(!self.video.shown)
        };
        let fullscreen = self.win.fullscreen;
        let (c, painted) = self.win.canvas(i);
        let want = Painted {
            w: c.w,
            h: c.h,
            mode,
        };
        let top = if *painted == want {
            controls::zone_top(c.h)
        } else {
            0
        };
        *painted = want;
        c.px[top * c.w..].fill(0); // see-through: the video subsurface is below
        let damage = Rect {
            x: 0,
            y: top,
            w: c.w,
            h: c.h - top,
        };
        let mut ui = self.ui.begin(c, dt);
        if mode != 0 {
            controls::blank(&mut ui, failed);
        }
        let look = Look {
            state: &self.player.state,
            visible: self.visible,
            fullscreen,
        };
        let acts = self.controls.draw(&mut ui, &look);
        ui.end();
        self.win.present(i, &[damage])?;
        self.cursor()?;
        for a in acts {
            self.act(a)?;
        }
        Ok(true)
    }

    /// Everything the bar shows, at the precision it shows it: the clock in whole
    /// seconds, the seek fill in window px. Hidden, it shows nothing.
    fn look_key(&self) -> [i64; 8] {
        if !self.visible {
            return [0; 8];
        }
        let s = &self.player.state;
        let fill = if s.duration > 0.0 {
            s.time / s.duration * self.win.width as f64
        } else {
            0.0
        };
        [
            s.time as i64,
            fill as i64,
            s.duration as i64,
            s.volume.round() as i64,
            i64::from(s.paused),
            i64::from(s.muted),
            i64::from(s.ended),
            i64::from(self.player.failed.is_some()), // the "can't play" screen
        ]
    }

    fn retitle(&mut self) -> Res<()> {
        let t = &self.player.state.title;
        if *t != self.titled {
            self.win.set_title(if t.is_empty() { "video" } else { t })?;
            self.titled = t.clone();
        }
        Ok(())
    }
}

impl App {
    /// Hand over clickable things, the arrow elsewhere, none while the bar hides.
    fn cursor(&mut self) -> Res<()> {
        if !self.visible {
            return Ok(self.win.hide_cursor()?);
        }
        let shape = if self.ui.pointer {
            cursor_shape::SHAPE_POINTER
        } else {
            cursor_shape::SHAPE_DEFAULT
        };
        Ok(self.win.set_cursor(shape)?)
    }
}
