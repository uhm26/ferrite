//! Mouse: text selection (drag, double-click for a word), wheel scrollback, and
//! reporting clicks/wheel/motion to apps that ask for it (modes 1000-1006).

use std::time::{Duration, Instant};

use super::{App, Error};
use crate::render::Selection;
use crate::term::cell::Flags;
use crate::term::{Mouse as Mode, Term};
use crate::window::Pointer;
use crate::wl::proto::pointer::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const WHEEL_LINES: i32 = 3;
/// Characters that count as part of a word for double-click (besides alphanumerics).
const WORD_CHARS: &str = "@-./_~?&=%+#:";

#[derive(Default)]
pub(super) struct Mouse {
    pub x: f64,
    y: f64,
    held: Option<u32>,
    anchor: Option<(u64, usize)>,
    head: (u64, usize),
    word: bool,
    last_click: Option<(Instant, (u64, usize))>,
    smooth: f64,
    /// Last cell reported to the app (motion is only sent when it changes).
    reported: Option<(usize, usize)>,
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
            let (cells, wrapped) = term.line_cells(line)?;
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
                if self.dragging_tab() {
                    self.drag_tab();
                } else {
                    self.motion();
                }
            }
            Pointer::Button { pressed: false, .. } if self.dragging_tab() => self.release_tab(),
            Pointer::Button {
                button,
                pressed: true,
            } if self.on_bar() => {
                return self.press_tab(button);
            }
            Pointer::Button { button, pressed } => {
                if pressed {
                    self.finish_rename()?; // not on release: a right-click rename starts on press
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
        let f = &self.renderer.font;
        let pad = self.renderer.pad as f64;
        let top = self.renderer.top as f64;
        let at = |v: f64, size: usize, max: usize| {
            (((v - pad) / size as f64).floor().max(0.0) as usize).min(max - 1)
        };
        (
            at(self.mouse.x, f.cell_w, self.tabs[self.active].term.cols()),
            at(
                self.mouse.y - top,
                f.cell_h,
                self.tabs[self.active].term.rows(),
            ),
        )
    }

    /// Line number (stable across scrolling) of visible row `y`.
    fn line_of(&self, y: usize) -> u64 {
        let t = &self.tabs[self.active].term;
        t.history_pushed() - t.view.min(t.history_len()) as u64 + y as u64
    }

    fn reporting(&self) -> bool {
        self.tabs[self.active].term.modes.mouse != Mode::Off
    }

    fn motion(&mut self) {
        let (col, row) = self.cell();
        if self.reporting() {
            let m = self.tabs[self.active].term.modes.mouse;
            let send = m == Mode::Motion || (m == Mode::Drag && self.mouse.held.is_some());
            if send && self.mouse.reported != Some((col, row)) {
                let b = self.mouse.held.map_or(3, button_code) + 32;
                self.report(b, (col, row), true);
            }
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
        if self.reporting() {
            self.report(button_code(button), (col, row), pressed);
            return Ok(());
        }
        if button == BTN_MIDDLE && pressed {
            // Middle click pastes the clipboard (we have no primary selection).
            return self.request_paste();
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
        if double {
            let (a, b) = word_bounds(&self.tabs[self.active].term, at);
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
        } else if self.tabs[self.active].term.is_alt()
            && self.tabs[self.active].term.modes.alt_scroll
        {
            let key: &[u8] = match (lines < 0, self.tabs[self.active].term.modes.app_cursor) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            for _ in 0..lines.unsigned_abs() {
                self.tabs[self.active].pty.write(key);
            }
        } else {
            self.tabs[self.active].term.scroll_view(-(lines as isize));
        }
    }

    /// Send one mouse event to the app, SGR (1006) or legacy encoding.
    fn report(&mut self, code: u8, (col, row): (usize, usize), pressed: bool) {
        self.mouse.reported = Some((col, row));
        let msg = if self.tabs[self.active].term.modes.mouse_sgr {
            let end = if pressed { 'M' } else { 'm' };
            format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes()
        } else {
            let b = if pressed { code } else { 3 };
            let enc = |v: usize| (v + 33).min(255) as u8;
            vec![0x1b, b'[', b'M', b + 32, enc(col), enc(row)]
        };
        self.tabs[self.active].pty.write(&msg);
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
