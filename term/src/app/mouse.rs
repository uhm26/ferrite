//! Mouse: text selection (drag, double-click for a word), wheel scrollback, and
//! reporting clicks/wheel/motion to apps that ask for it (modes 1000-1006).

use std::time::{Duration, Instant};

use super::session::Session;
use super::{App, Error};
use crate::render::Selection;
use crate::term::cell::Flags;
use crate::term::{Mouse as Mode, Term};
use crate::window::Pointer;
use crate::wl::proto::cursor_shape::{SHAPE_COL_RESIZE, SHAPE_DEFAULT, SHAPE_POINTER};
use crate::wl::proto::pointer::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const WHEEL_LINES: i32 = 3;
/// Characters that count as part of a word for double-click (besides alphanumerics).
const WORD_CHARS: &str = "@-./_~?&=%+#:";

#[derive(Default)]
pub(super) struct Mouse {
    pub x: f64,
    pub y: f64,
    held: Option<u32>,
    anchor: Option<(u64, usize)>,
    head: (u64, usize),
    word: bool,
    last_click: Option<(Instant, (u64, usize))>,
    smooth: f64,
    /// Last cell reported to the app (motion is only sent when it changes).
    reported: Option<(usize, usize)>,
    /// The clickable link or path under the pointer (in the split: `true`).
    pub hover: Option<(bool, Selection)>,
    /// (pane, col, row) the hover was last worked out for.
    pub hover_at: Option<(bool, usize, usize)>,
    /// The split divider is being dragged.
    pub divider: bool,
}

impl Mouse {
    pub fn selection(&self) -> Option<Selection> {
        let a = self.anchor?;
        if a == self.head && !self.word {
            return None;
        }
        let (start, end) = if a <= self.head {
            (a, self.head)
        } else {
            (self.head, a)
        };
        Some(Selection { start, end })
    }

    /// Returns whether there was a selection.
    pub fn clear_selection(&mut self) -> bool {
        let had = self.selection().is_some();
        self.anchor = None;
        self.word = false;
        had
    }

    pub fn selected_text(&self, term: &Term) -> Option<String> {
        let s = self.selection()?;
        let mut out = String::new();
        for line in s.start.0..=s.end.0 {
            let Some((cells, wrapped)) = term.line_cells(line) else {
                continue; // scrolled out of history: copy the rest
            };
            let from = if line == s.start.0 { s.start.1 } else { 0 };
            let to = if line == s.end.0 {
                s.end.1 + 1
            } else {
                cells.len()
            };
            let text: String = cells
                .get(from.min(cells.len())..to.min(cells.len()))
                .unwrap_or(&[])
                .iter()
                .filter(|c| !c.flags.has(Flags::SPACER))
                .map(|c| c.ch)
                .collect();
            let joined = wrapped && line != s.end.0;
            out.push_str(if joined { &text } else { text.trim_end() });
            if !joined && line != s.end.0 {
                out.push('\n');
            }
        }
        Some(out)
    }
}

impl App {
    /// Returns false when the last tab was closed (middle click on its tab).
    pub(super) fn pointer(&mut self, p: Pointer) -> Result<bool, Error> {
        match p {
            Pointer::Motion(x, y) => {
                (self.mouse.x, self.mouse.y) = (x, y);
                let hdr = self.renderer.top + self.renderer.bar_height();
                let over_header = self
                    .split_x()
                    .is_some_and(|sx| x >= sx as f64 && y < hdr as f64);
                self.update_hover();
                let over_link = self.mouse.hover.is_some();
                let clickable = y < self.renderer.top as f64 || over_header || over_link;
                let shape = match () {
                    _ if self.mouse.divider || self.on_divider(x, y) => SHAPE_COL_RESIZE,
                    _ if clickable => SHAPE_POINTER,
                    _ => SHAPE_DEFAULT,
                };
                self.win.set_cursor(shape);
                if self.mouse.divider {
                    self.drag_divider(x);
                } else if self.dragging_tab() {
                    self.drag_tab();
                } else {
                    self.motion();
                }
            }
            Pointer::Button { pressed: true, button: BTN_LEFT } if self.on_divider(self.mouse.x, self.mouse.y) => {
                self.mouse.divider = true;
            }
            Pointer::Button { pressed: false, .. } if self.mouse.divider => {
                self.mouse.divider = false;
                self.regrid(); // the panes take their final size now
            }
            Pointer::Button {
                pressed: false,
                button,
            } if self.tab_press == Some(button) => {
                self.release_tab();
            }
            Pointer::Button {
                button,
                pressed: true,
            } if self.on_bar() => {
                return self.press_tab(button);
            }
            Pointer::Button { pressed: true, .. }
                if self.press_split_header((self.mouse.x, self.mouse.y)) => {}
            Pointer::Button { button, pressed } => {
                if pressed {
                    self.finish_rename()?; // not on release: a right-click rename starts on press
                    if let Some(sx) = self.split_x() {
                        self.focus_pane(self.mouse.x >= sx as f64); // a click picks the pane
                    }
                }
                self.button(button, pressed)?;
            }
            Pointer::Scroll { discrete, px } => self.scroll(discrete, px),
        }
        Ok(true)
    }

    fn on_bar(&self) -> bool {
        self.mouse.y < self.renderer.top as f64 && self.mouse.held.is_none()
    }

    /// The cell under the pointer (col, row), clamped to the grid.
    fn cell(&self) -> (usize, usize) {
        self.cell_in(self.tabs[self.active].focus_split)
    }

    /// The active tab's split pane (`true`, if it has one) or its main pane.
    pub(super) fn pane_in(&self, split: bool) -> &Session {
        let t = &self.tabs[self.active];
        t.split.as_deref().filter(|_| split).unwrap_or(t)
    }

    /// The cell under the pointer (col, row) in a pane, clamped to its grid.
    pub(super) fn cell_in(&self, split: bool) -> (usize, usize) {
        let f = &self.renderer.font;
        let pad = self.renderer.pad as f64;
        // The pane's grid origin: the split sits right of its divider, below its header.
        let split = self.split_x().filter(|_| split);
        let left = split.unwrap_or(0) as f64;
        let hdr = if split.is_some() {
            self.renderer.bar_height()
        } else {
            0
        };
        let top = (self.renderer.top + hdr) as f64;
        let at = |v: f64, size: usize, max: usize| {
            (((v - pad) / size as f64).floor().max(0.0) as usize).min(max - 1)
        };
        let term = &self.pane_in(split.is_some()).term;
        (
            at(self.mouse.x - left, f.cell_w, term.cols()),
            at(self.mouse.y - top, f.cell_h, term.rows()),
        )
    }

    /// Line number (stable across scrolling) of visible row `y` in the focused pane.
    pub(super) fn line_of(&self, y: usize) -> u64 {
        line_in(&self.tabs[self.active].pane().term, y)
    }

    fn reporting(&self) -> bool {
        self.tabs[self.active].pane().term.modes.mouse != Mode::Off
    }

    fn motion(&mut self) {
        let (col, row) = self.cell();
        if self.reporting() {
            let m = self.tabs[self.active].pane().term.modes.mouse;
            let send = m == Mode::Motion || (m == Mode::Drag && self.mouse.held.is_some());
            if send && self.mouse.reported != Some((col, row)) {
                let b = self.mouse.held.map_or(3, button_code) + 32;
                self.report(b, (col, row), true);
            }
            return;
        }
        if self.mouse.held == Some(BTN_LEFT) && self.doc_click((col, row), true, false) {
            return;
        }
        if self.mouse.held == Some(BTN_LEFT) && self.mouse.anchor.is_some() {
            let head = (self.line_of(row), col);
            if head != self.mouse.head {
                self.mouse.head = head;
                self.redraw = true;
            }
        }
    }

    fn button(&mut self, button: u32, pressed: bool) -> Result<(), Error> {
        self.mouse.held = if pressed { Some(button) } else { None };
        let (col, row) = self.cell();
        // Ctrl+click opens a link or path, also over apps that take the mouse.
        if pressed && button == BTN_LEFT && self.win.ctrl_held() && self.ctrl_click((col, row)) {
            self.mouse.held = None;
            return Ok(());
        }
        if self.reporting() {
            self.report(button_code(button), (col, row), pressed);
            return Ok(());
        }
        if button == BTN_MIDDLE && pressed {
            // Middle click pastes the clipboard (we have no primary selection).
            super::input::log(self.request_paste());
            return Ok(());
        }
        if button != BTN_LEFT || !pressed {
            return Ok(());
        }
        let at = (self.line_of(row), col);
        let now = Instant::now();
        let double = self
            .mouse
            .last_click
            .is_some_and(|(t, p)| p == at && now - t < DOUBLE_CLICK);
        self.mouse.last_click = Some((now, at));
        if self.doc_click((col, row), false, double) {
            return Ok(());
        }
        if double {
            let (a, b) = word_bounds(&self.tabs[self.active].pane().term, at);
            self.mouse.anchor = Some(a);
            self.mouse.head = b;
            self.mouse.word = true;
        } else {
            self.mouse.anchor = Some(at);
            self.mouse.head = at;
            self.mouse.word = false;
        }
        self.redraw = true;
        Ok(())
    }

    fn scroll(&mut self, discrete: i32, px: f64) {
        let lines = if discrete != 0 {
            self.mouse.smooth = 0.0;
            discrete * WHEEL_LINES
        } else {
            self.mouse.smooth += px / self.renderer.font.cell_h as f64;
            let whole = self.mouse.smooth.trunc();
            self.mouse.smooth -= whole;
            whole as i32
        };
        if lines == 0 {
            return;
        }
        let cell = self.cell();
        if self.reporting() {
            let b = if lines < 0 { 64 } else { 65 };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.report(b, cell, true);
            }
        } else if self.tabs[self.active].pane().term.is_alt()
            && self.tabs[self.active].pane().term.modes.alt_scroll
        {
            let key: &[u8] = match (
                lines < 0,
                self.tabs[self.active].pane().term.modes.app_cursor,
            ) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            for _ in 0..lines.unsigned_abs() {
                self.tabs[self.active].pane_mut().write(key);
            }
        } else if !self.doc_scroll(lines as isize) {
            self.tabs[self.active]
                .pane_mut()
                .term
                .scroll_view(-(lines as isize));
        }
    }

    /// Send one mouse event to the app, SGR (1006) or legacy encoding.
    fn report(&mut self, code: u8, (col, row): (usize, usize), pressed: bool) {
        self.mouse.reported = Some((col, row));
        let msg = if self.tabs[self.active].pane().term.modes.mouse_sgr {
            let end = if pressed { 'M' } else { 'm' };
            format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes()
        } else {
            let b = if pressed { code } else { 3 };
            let enc = |v: usize| (v + 33).min(255) as u8;
            vec![0x1b, b'[', b'M', b + 32, enc(col), enc(row)]
        };
        self.tabs[self.active].pane_mut().write(&msg);
    }
}

fn button_code(button: u32) -> u8 {
    match button {
        BTN_LEFT => 0,
        BTN_MIDDLE => 1,
        BTN_RIGHT => 2,
        _ => 3,
    }
}

/// The word around `at` on its line: alphanumerics plus [`WORD_CHARS`].
fn word_bounds(term: &Term, at: (u64, usize)) -> ((u64, usize), (u64, usize)) {
    let Some((cells, _)) = term.line_cells(at.0) else {
        return (at, at);
    };
    let is_word = |i: usize| {
        cells
            .get(i)
            .is_some_and(|c| c.ch.is_alphanumeric() || WORD_CHARS.contains(c.ch))
    };
    if !is_word(at.1) {
        return (at, at);
    }
    let mut a = at.1;
    while a > 0 && is_word(a - 1) {
        a -= 1;
    }
    let mut b = at.1;
    while is_word(b + 1) {
        b += 1;
    }
    ((at.0, a), (at.0, b))
}

/// Line number (stable across scrolling) of visible row `y` in `t`.
pub(super) fn line_in(t: &Term, y: usize) -> u64 {
    t.history_pushed() - t.view.min(t.history_len()) as u64 + y as u64
}
