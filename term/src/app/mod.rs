//! The event loop: one thread, one poll over the compositor socket, every tab's
//! pty and (while pasting) the clipboard pipe. Shell output is parsed as soon as it
//! arrives; drawing happens at most once per frame callback, so a flood costs
//! parsing only. Tabs are in `tabs.rs`/`session.rs`, keys in `input.rs`, mouse in
//! `mouse.rs`.

mod input;
mod live;
mod mouse;
mod open;
mod restart;
mod session;
mod split;
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
pub(super) const REGRID_DELAY: Duration = Duration::from_millis(100);
const READY: i16 = POLLIN | POLLHUP | POLLERR;
/// Largest paste (text or image) we hold in memory.
const MAX_PASTE: usize = 64 << 20;

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
    /// Where the split divider sits, as a share of the window width (dragged).
    split_frac: f32,
    /// A paste in flight: the pipe, bytes so far, and the image mime if it is one.
    paste: Option<(File, Vec<u8>, Option<String>)>,
    mouse: mouse::Mouse,
    /// Window title last sent.
    title: String,
    stats: stats::Stats,
    /// Reused poll set: window, paste, then one slot per tab.
    fds: Vec<PollFd>,
    /// Mouse button held since a press on the tab bar (left: drag to move).
    tab_press: Option<u32>,
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
            split_frac: 0.5,
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
        let none = PollFd {
            fd: -1,
            events: 0,
            revents: 0,
        };
        fds.push(
            self.paste
                .as_ref()
                .map_or(none, |(f, _, _)| PollFd::new(f.as_fd(), POLLIN)),
        );
        fds.push(
            self.watch
                .as_ref()
                .map_or(none, |w| PollFd::new(w.fd(), POLLIN)),
        );
        // Two slots per tab: its main pane, then its split (or a skipped slot).
        // A viewer has no pty: its slot is skipped too.
        let slot = |s: &Session| {
            s.pty.as_ref().map_or(none, |p| {
                let out = if p.wants_write() { POLLOUT } else { 0 };
                PollFd::new(p.fd(), POLLIN | out)
            })
        };
        for s in &self.tabs {
            fds.push(slot(s));
            fds.push(s.split.as_deref().map_or(none, slot));
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
        let sync = self.tabs[self.active]
            .sync_since
            .map(|t| t + SYNC_TIMEOUT)
            .filter(|t| *t > now);
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
            // Poll slots: 3 + 2i main pane, 4 + 2i split; a tab opened this turn has none.
            let Some(fd) = self.fds.get(3 + 2 * i).copied() else {
                continue;
            };
            let (n, alive) = self.tabs[i].pump(fd, buf, budget);
            self.count(n, &mut budget);
            if !alive {
                if !self.main_exited(i)? {
                    return Ok(false);
                }
                continue;
            }
            let split_fd = self.fds.get(4 + 2 * i).copied();
            if let (Some(fd), Some(sp)) = (split_fd, self.tabs[i].split.as_mut()) {
                let (n, alive) = sp.pump(fd, buf, budget);
                self.count(n, &mut budget);
                if !alive {
                    self.close_split(i);
                }
            }
            let tab = &mut self.tabs[i];
            titles |= tab.settle() | tab.split.as_mut().is_some_and(|s| s.settle());
        }
        if titles {
            self.refresh_labels();
            self.update_title()?;
            self.redraw = true;
        }
        Ok(true)
    }

    /// Bookkeeping for `n` bytes just read: the shared budget and stats.
    fn count(&mut self, n: usize, budget: &mut usize) {
        *budget = budget.saturating_sub(n).max(64 << 10);
        self.stats.bytes += n as u64;
        if n > 0 {
            self.stats.output();
        }
    }

    /// Returns false when the window should go away.
    fn handle(&mut self, ev: Event) -> Result<bool, Error> {
        match ev {
            // A file with unsaved edits holds the window open once (its header says so).
            Event::Close if self.keep_unsaved(None) => {}
            Event::Close => return Ok(false),
            Event::Resize(w, h) => {
                // Every configure is acked; answer it with a commit even if nothing changed.
                self.resized = true;
                self.redraw = true;
                self.resize(w, h);
            }
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

    fn draw(&mut self) -> Result<(), Error> {
        let t = &self.tabs[self.active];
        let panes = || std::iter::once(t).chain(t.split.as_deref());
        let dirty = panes().any(|s| s.term.dirty);
        let waiting = self.win.frame_pending() && !self.resized;
        if !(dirty || self.redraw) || waiting || self.win.width == 0 {
            return Ok(());
        }
        if panes().any(|s| s.sync_since.is_some_and(|t| t.elapsed() < SYNC_TIMEOUT)) {
            return Ok(());
        }
        if panes().any(|s| s.term.recolor) {
            self.win.invalidate_buffers();
        }
        let Some(i) = self.win.acquire()? else {
            return Ok(()); // all buffers busy: retry on release
        };
        let t0 = Instant::now();
        let damage = self.paint(i);
        self.stats.add_frame(t0.elapsed());
        let t = &mut self.tabs[self.active];
        for s in std::iter::once(&mut t.term).chain(t.split.as_mut().map(|s| &mut s.term)) {
            (s.dirty, s.recolor) = (false, false);
        }
        self.redraw = false;
        let resized = std::mem::take(&mut self.resized);
        if !damage.is_empty() || resized {
            self.win.present(i, &damage)?;
            self.stats.presented();
        }
        Ok(())
    }

    /// Draw the active tab into buffer `i`: the main pane, then the split (right of
    /// a hairline, under its header). Keys go to one pane; only it shows a solid
    /// cursor and the selection. Returns damage.
    fn paint(&mut self, i: usize) -> Vec<crate::render::draw::Rect> {
        let split_x = self.split_x();
        let sel = self.mouse.selection();
        let hover = |split: bool| self.mouse.hover.filter(|h| h.0 == split).map(|h| h.1);
        let (main_hover, split_hover) = (hover(false), hover(true));
        let (mut canvas, shown) = self.win.canvas(i);
        let t = &self.tabs[self.active];
        let on_split = t.focus_split && split_x.is_some();
        let r = &mut self.renderer;
        let view = View {
            focused: self.focused && !on_split,
            selection: sel.filter(|_| !on_split),
            hover: main_hover,
            tabs: &self.labels,
            active_tab: self.active,
        };
        r.span = (0, split_x.map_or(usize::MAX, |x| x - 1));
        let mut damage = r.draw(&t.term, &view, &mut canvas, &mut shown[0]);
        if let (Some(x), Some(sp)) = (split_x, t.split.as_deref()) {
            let view = View {
                focused: self.focused && on_split,
                selection: sel.filter(|_| on_split),
                hover: split_hover,
                tabs: &[],
                active_tab: 0,
            };
            let (top, w) = (r.top, canvas.w);
            (r.span, r.top) = ((x, usize::MAX), top + r.bar_height());
            damage.extend(r.draw(&sp.term, &view, &mut canvas, &mut shown[1]));
            r.top = top;
            let doc = sp.doc.as_ref();
            let title = sp.label();
            let header = crate::render::Header {
                term: &sp.term,
                title: &title,
                focused: on_split,
                dirty: doc.is_some_and(|d| d.buf.dirty()),
                note: doc.map_or("", |d| d.note.as_str()),
                toggle: doc.and_then(crate::view::Doc::toggle),
            };
            damage.extend(r.pane_header(&mut canvas, &header, (x, w), &mut shown[1]));
        }
        r.span = (0, usize::MAX);
        damage
    }

    fn read_paste(&mut self) {
        let Some((f, data, _)) = &mut self.paste else {
            return;
        };
        let mut chunk = [0u8; 16 << 10];
        loop {
            match f.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) if data.len() + n > MAX_PASTE => {
                    eprintln!("term: paste over {} MiB dropped", MAX_PASTE >> 20);
                    self.paste = None;
                    return;
                }
                Ok(n) => data.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(_) => break,
            }
        }
        if let Some((_, data, image)) = self.paste.take() {
            self.finish_paste(&data, image);
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
