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
use frame::window::{
    ACTION_COPY, ACTION_MOVE, Canvas, Drag, DragIcon, Event, Pointer, Rect, Window,
};
use frame::wl::Error;
use frame::wl::proto::{cursor_shape, pointer};

use crate::browse::Browser;
use crate::browse::tabs::Tabs;
use crate::view::{Pick, View};

/// Longest step a spring takes after a stall, s (no jumps after a hiccup).
const MAX_DT: f32 = 0.05;
/// linux/input-event-codes.h: the mouse side buttons.
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;
const BTN_MIDDLE: u32 = 0x112;
/// How often a running job's progress is redrawn, ms.
const PROGRESS_MS: i32 = 100;
/// Smallest window: the folded sidebar, the toolbar and a Name + Size list still fit
/// (owner 2026-11-01: windows could be resized "WAY too much").
const MIN_SIZE: (usize, usize) = (520, 400);
/// What files are dragged and dropped as.
const URI_LIST: &str = "text/uri-list";

/// A pipe being read: what came so far.
type Pipe = (File, Vec<u8>);

struct App {
    win: Window,
    ui: State,
    view: View,
    tabs: Tabs,
    last: Instant,
    /// The folder the window title names.
    titled: PathBuf,
    /// Live refresh: inotify on the folders in `watching` (all tabs').
    watch: Option<Watch>,
    watching: Vec<PathBuf>,
    /// A paste being read: the pipe from the clipboard owner and what came so far.
    paste: Option<Pipe>,
    /// Files dropped from another app being read, the folder they go to, and
    /// whether they are copied.
    dropping: Option<(Pipe, PathBuf, bool)>,
}

/// Open the window on the folder given as an argument, else home (like Nemo;
/// `files .` for here). `--png` renders headless instead (see [`shot`]), `--portal`
/// serves the file picker portal, `--pick` is one picker window (view/pick.rs).
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let flags: Vec<&str> = args.iter().filter_map(|a| a.to_str()).collect();
    if flags.contains(&"--png") {
        return shot(&args);
    }
    if flags.contains(&"--portal") {
        return crate::portal::serve();
    }
    let pick = Pick::from_args(&flags);
    let start = positional(&args, Pick::TAKES_VALUE).unwrap_or_else(crate::view::home);
    let mut app = App::new(start)?;
    if let Some((p, title)) = pick {
        app.view.set_pick(p);
        app.win.set_title(&title)?;
    }
    app.run()?;
    if flags.contains(&"--pick") {
        let Some(paths) = app.view.picked() else {
            std::process::exit(1); // cancelled: the portal answers so
        };
        let mut out = Vec::new();
        for p in paths {
            out.extend_from_slice(p.as_os_str().as_encoded_bytes());
            out.push(0);
        }
        std::io::Write::write_all(&mut std::io::stdout(), &out)?;
    }
    Ok(())
}

/// The one argument that is neither a flag nor a flag's value (the folder).
fn positional(args: &[OsString], takes_value: &[&str]) -> Option<PathBuf> {
    let flag = |a: &OsString| a.to_str().is_some_and(|s| s.starts_with("--"));
    let value = |i: usize| {
        i > 0
            && args[i - 1]
                .to_str()
                .is_some_and(|f| takes_value.contains(&f))
    };
    (0..args.len())
        .filter(|&i| !flag(&args[i]) && !value(i))
        .map(|i| PathBuf::from(&args[i]))
        .next_back()
}

impl App {
    fn new(start: PathBuf) -> Result<App, Box<dyn std::error::Error>> {
        let mut win: Window = Window::new("files", "files", true)?;
        let (w, h) = win.configured_size();
        win.set_size_limits(MIN_SIZE, (0, 0))?; // 0: no largest size
        win.set_size(if w == 0 { 720 } else { w }, if h == 0 { 560 } else { h });
        Ok(App {
            win,
            ui: State::new(Text::new(theme::FAMILY)?),
            view: View::new()?,
            // A folder we can't resolve still opens: Browser falls back and shows why.
            tabs: Tabs::new(Browser::first(std::fs::canonicalize(&start).unwrap_or(start))),
            last: Instant::now(),
            titled: PathBuf::new(),
            watch: None,
            watching: Vec::new(),
            paste: None,
            dropping: None,
        })
    }
    fn run(&mut self) -> Result<(), Error> {
        let mut dirty = true;
        loop {
            while let Some(ev) = self.win.next_event() {
                dirty |= self.on_event(ev)?;
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
    fn on_event(&mut self, ev: Event) -> Result<bool, Error> {
        let i = &mut self.ui.input;
        match ev {
            Event::Close => self.view.quit = true,
            Event::Drag(d) => self.on_drag(d)?,
            Event::Resize(w, h) if w > 0 && h > 0 => self.win.set_size(w, h),
            Event::Key(k) => i.keys.push(k),
            Event::Mods(m) => i.mods = m,
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
            Event::Pointer(Pointer::Button {
                button: BTN_MIDDLE,
                pressed: true,
            }) => {
                let pos = i.pos;
                self.view.middle_click(pos);
            }
            Event::Pointer(Pointer::Scroll { discrete, px }) => {
                self.view.scroll_by(discrete, px, i.mods.ctrl);
            }
            // Mouse side buttons: back / forward, like Nemo.
            Event::Pointer(Pointer::Button {
                button: BTN_SIDE,
                pressed: true,
            }) => self.tabs.cur_mut().travel(true),
            Event::Pointer(Pointer::Button {
                button: BTN_EXTRA,
                pressed: true,
            }) => self.tabs.cur_mut().travel(false),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Sleep until the compositor sends something (a frame callback while animating).
    /// Also wakes when the shown folder changes on disk; then it is read again
    /// and this returns true (needs a redraw).
    fn wait(&mut self) -> Result<bool, Error> {
        self.win.flush()?;
        // Every open tab's folder: a pane beside or a tab behind must not show files
        // that moved away (owner: an undone drop "just disappeared").
        let mut want: Vec<PathBuf> = self.tabs.each().map(|(_, p)| p.clone()).collect();
        want.sort();
        want.dedup();
        if self.watching != want {
            let dirs: Vec<&std::path::Path> = want.iter().map(PathBuf::as_path).collect();
            // shortcut: no live refresh where inotify fails (some network mounts).
            self.watch = Watch::new(&dirs).ok();
            self.watching = want;
        }
        let mut timeout = self.win.next_deadline().map_or(-1, |at| {
            at.saturating_duration_since(Instant::now()).as_millis() as i32
        });
        // A job running: wake ~10×/s for its progress (idle stays at 0 CPU).
        let busy = self.view.busy() || self.tabs.cur().searching();
        if busy && !(0..=PROGRESS_MS).contains(&timeout) {
            timeout = PROGRESS_MS;
        }
        let mut fds = vec![PollFd::new(self.win.fd(), POLLIN)];
        // Optional fds after the window: the folder watch, then a paste in flight.
        let watch_at = self.watch.as_ref().map(|w| {
            fds.push(PollFd::new(w.fd(), POLLIN));
            fds.len() - 1
        });
        let mut pipe_at = |p: Option<&File>| {
            p.map(|f| {
                fds.push(PollFd::new(f.as_fd(), POLLIN | POLLHUP));
                fds.len() - 1
            })
        };
        let paste_at = pipe_at(self.paste.as_ref().map(|p| &p.0));
        let drop_at = pipe_at(self.dropping.as_ref().map(|d| &d.0.0));
        fds.push(PollFd::new(self.view.jobs_fd(), POLLIN));
        let jobs_at = fds.len() - 1;
        fds.push(PollFd::new(self.view.thumbs_fd(), POLLIN));
        let thumbs_at = fds.len() - 1;
        fds.push(PollFd::new(self.view.drives_fd(), POLLIN));
        let drives_at = fds.len() - 1;
        sys::wait(&mut fds, timeout)?;
        let ready = |at: Option<usize>| at.is_some_and(|i| fds[i].ready(POLLIN | POLLHUP));
        let (changed, pasting, dropped) = (ready(watch_at), ready(paste_at), ready(drop_at));
        let jobs_done = fds[jobs_at].ready(POLLIN);
        let thumbs = fds[thumbs_at].ready(POLLIN);
        let drives = fds[drives_at].ready(POLLIN);
        drop(fds);
        self.win.dispatch()?;
        self.win.tick(Instant::now());
        let pasted = pasting && self.read_paste();
        let dropped = dropped && self.read_drop()?;
        let pictures = thumbs && self.view.thumbs_ready();
        let found = self.tabs.cur_mut().search_tick();
        if drives {
            self.view.drive_reports(self.tabs.cur_mut());
        }
        if jobs_done {
            self.view.job_reports(self.tabs.cur_mut());
        }
        if changed && self.watch.as_mut().is_some_and(Watch::drain) {
            self.tabs.reload_all();
            // Re-arm: if the folder was removed and made again, the old watch is dead.
            self.watching.clear();
            return Ok(true);
        }
        Ok(pasted || dropped || jobs_done || pictures || found || drives || busy)
    }

    /// Read what the clipboard owner wrote into the paste pipe. At the end (EOF or an
    /// error) hand the text to the view; true then (needs a redraw).
    fn read_paste(&mut self) -> bool {
        if !self.paste.as_mut().is_some_and(drain) {
            return false;
        }
        let text = self
            .paste
            .take()
            .map(|(_, b)| String::from_utf8_lossy(&b).into_owned());
        self.view.paste(text.unwrap_or_default());
        true
    }

    /// Drag and drop over the window, or our own drag ending.
    fn on_drag(&mut self, d: Drag) -> Result<(), Error> {
        let i = &mut self.ui.input;
        match d {
            Drag::Enter { mimes, at } => {
                i.pos = (at.0 as f32, at.1 as f32);
                self.view.drag_over(mimes.iter().any(|m| m == URI_LIST));
            }
            Drag::Motion(x, y) => i.pos = (x as f32, y as f32),
            Drag::Leave => self.view.drag_over(false),
            Drag::Drop => return self.drop(),
            Drag::Ended(_) => {
                // The press that started it never sees its release (the drag took it).
                i.down = false;
                self.view.drag_ended();
                return Ok(());
            }
        }
        let target = self.view.drop_target(&self.tabs, i.pos).is_some();
        // Ours moves unless Ctrl is held; other apps' files are copied.
        let preferred = if self.view.dragging_ours() && !i.mods.ctrl {
            ACTION_MOVE
        } else {
            ACTION_COPY
        };
        let mime = target.then_some(URI_LIST);
        self.win
            .drag_accept(mime, ACTION_COPY | ACTION_MOVE, preferred)
    }

    /// Something dropped on us: our own files go at once, other apps' are read first.
    fn drop(&mut self) -> Result<(), Error> {
        let pos = self.ui.input.pos;
        self.ui.input.down = false;
        let target = self.view.drop_target(&self.tabs, pos).map(|t| t.1);
        self.view.drag_over(false);
        if self.view.dragging_ours() {
            // Ours: move, Ctrl copies. We decide: the compositor reported copy (live, 2026-11-01).
            let copy = self.ui.input.mods.ctrl;
            self.win.drop_finish()?;
            if let Some(dir) = target {
                self.view.drop_ours(self.tabs.cur_mut(), dir, copy);
            }
            return Ok(());
        }
        let Some(dir) = target else {
            return self.win.drop_finish();
        };
        let copy = self.win.drop_action() != ACTION_MOVE;
        match self.win.drop_receive(URI_LIST)? {
            Some(fd) => self.dropping = Some(((File::from(fd), Vec::new()), dir, copy)),
            None => self.win.drop_finish()?,
        }
        Ok(())
    }

    /// Read the dropped uri list; at its end, copy or move the files. True then.
    fn read_drop(&mut self) -> Result<bool, Error> {
        if !self.dropping.as_mut().is_some_and(|d| drain(&mut d.0)) {
            return Ok(false);
        }
        self.win.drop_finish()?;
        if let Some(((_, bytes), dir, copy)) = self.dropping.take() {
            let paths = crate::uri::parse_list(&String::from_utf8_lossy(&bytes));
            self.view.drop_files(self.tabs.cur_mut(), paths, dir, copy);
        }
        Ok(true)
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
        self.view.draw(&mut ui, &mut self.tabs);
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
        if let Some(d) = self.view.take_drag(&mut self.ui) {
            // The pill sits below-right of the arrow tip, clear of the drop target.
            let icon = DragIcon {
                px: &d.px,
                w: d.w,
                h: d.h,
                at: (12, 16),
            };
            let all = ACTION_COPY | ACTION_MOVE;
            self.win
                .start_drag((&[URI_LIST], d.list), all, Some(icon))?;
        }
        if self.view.pick_title().is_none() && self.titled != self.tabs.cur().cwd {
            self.titled.clone_from(&self.tabs.cur().cwd);
            let name = self
                .titled
                .file_name()
                .map_or("/".into(), |n| n.to_string_lossy());
            self.win.set_title(&name)?;
        }
        Ok(true)
    }
}

/// Read what is in the pipe now. True at its end (EOF or an error).
fn drain((file, buf): &mut Pipe) -> bool {
    let mut chunk = [0u8; 4096];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => return true,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return false,
            Err(_) => return true,
        }
    }
}

/// Render settled frames into a PNG, no compositor needed, for checking the look:
/// `files --png out.png [--size WxH] [--grid] [--select N] [--pointer X,Y] [--menu X,Y] [--info]
/// [--tab FOLDER] [--split] [--zoom N] [--search TEXT] [folder]` (`--tab` opens a second tab and shows it; `--zoom` 0-3
/// sizes grid tiles)
/// (`--menu` right-clicks there; `--pointer` can then hover the menu).
fn shot(args_os: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<&str> = args_os.iter().filter_map(|a| a.to_str()).collect();
    let val = |f: &str| value(&args, f);
    let pair = |f: &str, sep: char| -> Option<(f32, f32)> {
        let (a, b) = val(f)?.split_once(sep)?;
        Some((a.parse().ok()?, b.parse().ok()?))
    };
    let (w, h) = pair("--size", 'x').unwrap_or((960.0, 600.0));
    let (w, h) = (w as usize, h as usize);
    let folder = positional(args_os, SHOT_VALUES).unwrap_or_else(crate::view::home);
    let mut tabs = shot_tabs(&args, folder);
    let mut view = View::new()?;
    let zoom = val("--zoom").and_then(|z| z.parse().ok()).unwrap_or(0);
    view.set_grid(args.contains(&"--grid"), zoom);
    view.set_info(args.contains(&"--info"));
    if let Some((p, _)) = Pick::from_args(&args) {
        view.set_pick(p);
    }
    let mut ui = State::new(Text::new(theme::FAMILY)?);
    ui.input.pos = pair("--pointer", ',').unwrap_or((-1.0, -1.0));
    if let Some(at) = pair("--menu", ',') {
        view.right_click(at);
    }
    let mut px = vec![0u32; w * h];
    for _ in 0..40 {
        let mut f = ui.begin(Canvas { px: &mut px, w, h }, MAX_DT);
        view.draw(&mut f, &mut tabs);
        f.end();
        // Thumbnails and search results come from worker threads: give them a moment.
        std::thread::sleep(std::time::Duration::from_millis(20));
        view.thumbs_ready();
        tabs.cur_mut().search_tick();
        view.drive_reports(tabs.cur_mut());
    }
    frame::png::write(val("--png").unwrap_or("files.png"), w, h, &px)?;
    Ok(())
}

/// Flags of `--png` that take a value (the folder is the argument that is neither).
const SHOT_VALUES: &[&str] = &[
    "--png",
    "--size",
    "--select",
    "--pointer",
    "--menu",
    "--tab",
    "--zoom",
    "--search",
    "--pick",
    "--name",
    "--title",
    "--accept",
];

/// The value after flag `f`.
fn value<'a>(args: &[&'a str], f: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| *a == f)?;
    args.get(i + 1).copied()
}

/// The tabs a headless shot shows: `folder`, `--tab`, `--split`, `--search`, `--select`.
fn shot_tabs(args: &[&str], folder: PathBuf) -> Tabs {
    let mut tabs = Tabs::new(Browser::new(
        std::fs::canonicalize(&folder).unwrap_or(folder),
    ));
    if let Some(t) = value(args, "--tab") {
        tabs.open(PathBuf::from(t), true);
    }
    if args.contains(&"--split") {
        tabs.split_with(0);
    }
    if let Some(s) = value(args, "--search") {
        tabs.cur_mut().search(s);
    }
    tabs.cur_mut().sel = value(args, "--select")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    tabs
}
