//! The event loop: one thread, one poll over the compositor socket, every tab's
//! pty and (while pasting) the clipboard pipe. Shell output is parsed as soon as it
//! arrives; drawing happens at most once per frame callback, so a flood costs
//! parsing only. Tabs are in `tabs.rs`/`session.rs`, keys in `input.rs`, mouse in
//! `mouse.rs`.

mod input;
mod live;
mod mouse;
mod restart;
mod session;
mod stats;
mod tabbar;
mod tabs;

pub use live::load as settings;

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::font::{self, Font};
use crate::pty;
use crate::render::{Renderer, View};
use crate::sys::{self, POLLERR, POLLHUP, POLLIN, POLLOUT, PollFd};
use crate::window::{Event, Window};
use crate::wl;
use session::Session;

pub use restart::RESTORE_ENV;

/// Most pty bytes handled per wakeup, over all tabs (input stays responsive
/// during floods).
const READ_BUDGET: usize = 4 << 20;
/// Longest an app may hold drawing with synchronized output (mode 2026).
const SYNC_TIMEOUT: Duration = Duration::from_millis(150);
/// Quiet time after the last resize step before grids reflow and apps get SIGWINCH
/// (foot's default resize-delay-ms is 100).
const REGRID_DELAY: Duration = Duration::from_millis(100);
const READY: i16 = POLLIN | POLLHUP | POLLERR;

#[derive(Debug)]
pub enum Error {
    Font(font::Error),
    Pty(pty::Error),
    Wayland(wl::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Font(e) => write!(f, "font: {e}"),
            Error::Pty(e) => write!(f, "{e}"),
            Error::Wayland(e) => write!(f, "wayland: {e}"),
        }
    }
}

impl From<wl::Error> for Error {
    fn from(e: wl::Error) -> Self {
        Error::Wayland(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Wayland(wl::Error::Io(e))
    }
}

pub struct App {
    cfg: Config,
    tabs: Vec<Session>,
    active: usize,
    /// Tab bar titles, rebuilt when a tab opens, closes or retitles.
    labels: Vec<String>,
    win: Window,
    renderer: Renderer,
    focused: bool,
    /// Something outside the terminal changed (focus, selection, tabs): draw.
    redraw: bool,
    /// The window changed size: draw now, without waiting for the frame callback,
    /// so the compositor never has to show a stale buffer stretched.
    resized: bool,
    /// When the grids follow the window size (see `resize`).
    regrid_at: Option<Instant>,
    paste: Option<(File, Vec<u8>)>,
    mouse: mouse::Mouse,
    /// Window title last sent.
    title: String,
    stats: stats::Stats,
    /// Reused poll set: window, paste, then one slot per tab.
    fds: Vec<PollFd>,
    /// A tab held with the left button (drag to move, click to rename).
    tab_press: Option<tabbar::TabPress>,
    /// Config file + rice glass switch, for live reloads.
    watch: Option<crate::sys::watch::Watch>,
}

impl App {
    /// `command` empty: the configured shell (fish if installed).
    pub fn new(cfg: Config, command: Vec<String>) -> Result<Self, Error> {
        let stats = stats::Stats::default(); // first: startup counts from here
        let font = Font::new(&cfg.font, cfg.font_size).map_err(Error::Font)?;
        let renderer = Renderer::new(font, cfg.padding, cfg.theme);
        let win = Window::new("term", "term", cfg.theme.opacity >= 1.0)?;
        let mut app = App {
            cfg,
            tabs: Vec::new(),
            active: 0,
            labels: Vec::new(),
            win,
            renderer,
            focused: true,
            redraw: true,
            resized: false,
            regrid_at: None,
            paste: None,
            mouse: mouse::Mouse::default(),
            title: String::new(),
            stats,
            fds: Vec::new(),
            tab_press: None,
            watch: live::watch(),
        };
        if let Some(saved) = restart::take_saved() {
            app.restore(saved)?;
        }
        if app.tabs.is_empty() {
            app.first_tab(command)?;
        }
        app.refresh_labels();
        Ok(app)
    }

    fn first_tab(&mut self, command: Vec<String>) -> Result<(), Error> {
        let argv = if command.is_empty() {
            vec![self.shell()]
        } else {
            command
        };
        // Start the shell at the real size, so its first output is laid out right.
        let (w, h) = self.win.configured_size();
        let (w, h) = self.window_size(w, h);
        let size = self.renderer.grid_for(w, h);
        let first = Session::spawn(&self.cfg, &argv, None, size).map_err(Error::Pty)?;
        self.tabs.push(first);
        Ok(())
    }

    /// Take over the tabs of the term that exec'd us (restart in place).
    fn restore(&mut self, saved: restart::Saved) -> Result<(), Error> {
        if saved.font_size != self.renderer.font.size {
            self.renderer.set_font_size(saved.font_size);
        }
        let active = saved.active;
        self.tabs = restart::sessions(&self.cfg, saved);
        self.active = active.min(self.tabs.len().saturating_sub(1));
        // The bar height only; the first configure sizes every tab.
        if self.tabs.len() > 1 {
            self.renderer.top = self.renderer.bar_height();
        }
        if !self.tabs.is_empty() {
            self.update_title()?;
        }
        Ok(())
    }

    fn shell(&self) -> String {
        self.cfg.shell.clone().unwrap_or_else(pty::default_shell)
    }

    fn cur(&mut self) -> &mut Session {
        &mut self.tabs[self.active]
    }

    /// Run until the last tab exits or the window is closed.
    pub fn run(&mut self) -> Result<(), Error> {
        let result = self.run_loop();
        if std::env::var_os("TERM_STATS").is_some() {
            eprintln!("{}", self.stats.report(&self.memory_report()));
        }
        result
    }

    fn run_loop(&mut self) -> Result<(), Error> {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            self.poll()?;
            if self.fds[0].ready(READY) {
                self.win.dispatch()?;
            }
            self.win.tick(Instant::now());
            while let Some(ev) = self.win.next_event() {
                if !self.handle(ev)? {
                    return Ok(());
                }
            }
            if self.fds[1].ready(READY) {
                self.read_paste();
            }
            if self.fds[2].ready(READY) && self.watch.as_mut().is_some_and(|w| w.drain()) {
                self.reload();
            }
            if self.regrid_at.is_some_and(|t| Instant::now() >= t) {
                self.regrid();
            }
            if !self.read_tabs(&mut buf)? {
                return Ok(());
            }
            self.draw()?;
            self.win.flush()?;
        }
    }

    /// Wait for the compositor, a paste, any tab's output, or a deadline.
    fn poll(&mut self) -> Result<(), Error> {
        let timeout = self.timeout_ms();
        let mut fds = std::mem::take(&mut self.fds);
        fds.clear();
        fds.push(PollFd::new(self.win.fd(), POLLIN));
        // fd -1 makes poll skip a slot: no paste running, or no live settings.
        let none = PollFd { fd: -1, events: 0, revents: 0 };
        fds.push(self.paste.as_ref().map_or(none, |(f, _)| PollFd::new(f.as_fd(), POLLIN)));
        fds.push(self.watch.as_ref().map_or(none, |w| PollFd::new(w.fd(), POLLIN)));
        for s in &self.tabs {
            let out = if s.pty.wants_write() { POLLOUT } else { 0 };
            fds.push(PollFd::new(s.pty.fd(), POLLIN | out));
        }
        let r = sys::wait(&mut fds, timeout);
        self.fds = fds;
        Ok(r?)
    }

    /// Poll timeout: key repeat or a held synchronized update, else forever.
    fn timeout_ms(&self) -> i32 {
        if self.win.has_events() {
            return 0; // e.g. the first configure, queued while connecting
        }
        let now = Instant::now();
        let sync = self.tabs[self.active].sync_since.map(|t| t + SYNC_TIMEOUT).filter(|t| *t > now);
        let soonest = [self.win.next_deadline(), sync, self.regrid_at];
        match soonest.into_iter().flatten().min() {
            Some(t) => t.saturating_duration_since(now).as_millis().min(1000) as i32 + 1,
            None => -1,
        }
    }

    /// Read every tab that has output; close tabs whose shell exited. Returns
    /// false when no tab is left.
    fn read_tabs(&mut self, buf: &mut [u8]) -> Result<bool, Error> {
        let mut budget = READ_BUDGET;
        let mut titles = false;
        let mut i = self.tabs.len();
        while i > 0 {
            i -= 1;
            // Poll slots for tabs start at 3; a tab opened this turn has none yet.
            let Some(fd) = self.fds.get(i + 3).copied() else {
                continue;
            };
            let s = &mut self.tabs[i];
            if fd.ready(POLLOUT) {
                s.pty.flush();
            }
            if fd.ready(READY) {
                let (n, alive) = s.read(buf, budget);
                budget = budget.saturating_sub(n).max(64 << 10);
                self.stats.bytes += n as u64;
                if !alive {
                    if !self.close_tab(i)? {
                        return Ok(false);
                    }
                    continue;
                }
            }
            titles |= self.tabs[i].settle();
        }
        if titles {
            self.refresh_labels();
            self.update_title()?;
            self.redraw = true;
        }
        Ok(true)
    }

    /// Returns false when the window should go away.
    fn handle(&mut self, ev: Event) -> Result<bool, Error> {
        match ev {
            Event::Close => return Ok(false),
            Event::Resize(w, h) => self.resize(w, h),
            Event::Frame | Event::Released => {}
            Event::Key(k) => return self.key(&k),
            Event::Focus(on) => {
                self.focused = on;
                self.redraw = true;
                self.cur().focus(on);
            }
            Event::Pointer(p) => return self.pointer(p),
        }
        Ok(true)
    }

    /// The window size to use for a configure of `w`×`h`. 0 = we pick: keep the size
    /// we have, or 80×24 cells before the first one.
    fn window_size(&self, w: usize, h: usize) -> (usize, usize) {
        let r = &self.renderer;
        let pad = 2 * r.pad;
        let pick = |given: usize, have: usize, default: usize| match (given, have) {
            (0, 0) => default,
            (0, have) => have,
            (given, _) => given,
        };
        let w = pick(w, self.win.width, 80 * r.font.cell_w + pad);
        let h = pick(h, self.win.height, 24 * r.font.cell_h + pad + r.top);
        (w, h)
    }

    /// A configure: the window repaints at the new size right away, but the grid
    /// (reflow) and the apps (SIGWINCH) only follow once resizing pauses, so a drag
    /// costs one repaint per step instead of a reflow plus a full app redraw.
    fn resize(&mut self, w: usize, h: usize) {
        let (w, h) = self.window_size(w, h);
        if (w, h) == (self.win.width, self.win.height) {
            return;
        }
        self.resized = true;
        self.win.set_size(w, h);
        self.regrid_at = Some(Instant::now() + REGRID_DELAY);
        self.redraw = true;
    }

    /// Fit every tab's grid to the window now and tell the apps.
    fn regrid(&mut self) {
        self.regrid_at = None;
        let (cw, ch) = (self.renderer.font.cell_w, self.renderer.font.cell_h);
        let (cols, rows) = self.renderer.grid_for(self.win.width, self.win.height);
        for s in &mut self.tabs {
            s.resize(cols, rows, (cw, ch));
        }
        self.mouse.clear_selection();
        self.redraw = true;
    }

    fn draw(&mut self) -> Result<(), Error> {
        let s = &self.tabs[self.active];
        let waiting = self.win.frame_pending() && !self.resized;
        if !(s.term.dirty || self.redraw) || waiting || self.win.width == 0 {
            return Ok(());
        }
        if s.sync_since.is_some_and(|t| t.elapsed() < SYNC_TIMEOUT) {
            return Ok(());
        }
        if s.term.recolor {
            self.win.invalidate_buffers();
            self.tabs[self.active].term.recolor = false;
        }
        let Some(i) = self.win.acquire()? else {
            return Ok(()); // all buffers busy: retry on release
        };
        let view = View {
            focused: self.focused,
            selection: self.mouse.selection(),
            tabs: &self.labels,
            active_tab: self.active,
        };
        let (mut canvas, shown) = self.win.canvas(i);
        let t0 = Instant::now();
        let term = &self.tabs[self.active].term;
        let damage = self.renderer.draw(term, &view, &mut canvas, shown);
        self.stats.add_frame(t0.elapsed());
        self.tabs[self.active].term.dirty = false;
        self.redraw = false;
        self.resized = false;
        if !damage.is_empty() {
            self.win.present(i, &damage)?;
        }
        Ok(())
    }

    fn read_paste(&mut self) {
        let Some((f, data)) = &mut self.paste else {
            return;
        };
        let mut chunk = [0u8; 16 << 10];
        loop {
            match f.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(_) => break,
            }
        }
        if let Some((_, data)) = self.paste.take() {
            self.paste_bytes(&data);
        }
    }

    /// Memory we hold on purpose (printed with `TERM_STATS=1`).
    pub fn memory_report(&self) -> String {
        let lines: usize = self.tabs.iter().map(|s| s.term.history_len()).sum();
        format!(
            "window {}x{}, {} tabs, buffers {} KiB, glyphs {} KiB, scrollback {lines} lines",
            self.win.width,
            self.win.height,
            self.tabs.len(),
            self.win.buffer_bytes() / 1024,
            self.renderer.glyph_bytes() / 1024,
        )
    }

    fn set_font_size(&mut self, size: f32) {
        let size = size.clamp(4.0, 72.0);
        self.renderer.set_font_size(size);
        if self.renderer.top > 0 {
            self.renderer.top = self.renderer.bar_height();
        }
        self.win.invalidate_buffers();
        let (w, h) = (self.win.width, self.win.height);
        self.resize(w, h);
        self.regrid();
        sys::trim_heap();
    }
}
