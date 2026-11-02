//! The split panel: a second pane right of a tab's main pane, a shell or a file
//! (Preview/Edit), under a compact header (name, unsaved dot, Preview/Edit switch,
//! ×). It opens when a command or a text file is sent to it (Ctrl+click); a click
//! on a pane gives it the keys; the divider between them drags.

use std::time::Instant;

use super::session::Session;
use super::{App, Error, REGRID_DELAY};
use crate::render::Hit;
use crate::view::{Mode, Outcome};
use crate::xkb::Key;

/// How far from the divider (px, either side) a press grabs it.
const GRAB: f64 = 4.0;
/// Narrowest a pane gets, in cells.
const MIN_CELLS: usize = 12;

impl App {
    /// x where a split pane starts (the 1 px divider is just left of it).
    fn split_at(&self) -> usize {
        let w = self.win.width;
        let min = (MIN_CELLS * self.renderer.font.cell_w + 2 * self.renderer.pad).min(w / 2);
        ((w as f32 * self.split_frac) as usize).clamp(min, w - min) + 1
    }

    /// x where the active tab's split pane starts, if it has one.
    pub(super) fn split_x(&self) -> Option<usize> {
        self.tabs[self.active]
            .split
            .as_ref()
            .map(|_| self.split_at())
    }

    /// Grid that fits a pane `w` px wide whose grid starts `top` px down.
    fn grid_in(&self, w: usize, top: usize) -> (usize, usize) {
        let r = &self.renderer;
        let cols = w.saturating_sub(2 * r.pad) / r.font.cell_w;
        let rows = self.win.height.saturating_sub(top + 2 * r.pad) / r.font.cell_h;
        (cols.max(1), rows.max(1))
    }

    /// Fit every tab (and its split) to the window now and tell the apps.
    pub(super) fn regrid(&mut self) {
        self.regrid_at = None;
        let (cw, ch) = (self.renderer.font.cell_w, self.renderer.font.cell_h);
        let (w, top, sx) = (self.win.width, self.renderer.top, self.split_at());
        let full = self.grid_in(w, top);
        let main = self.grid_in(sx - 1, top);
        let side = self.grid_in(w.saturating_sub(sx), top + self.renderer.bar_height());
        for s in &mut self.tabs {
            let m = if s.split.is_some() { main } else { full };
            s.resize(m.0, m.1, (cw, ch));
            if let Some(sp) = s.split.as_mut() {
                sp.resize(side.0, side.1, (cw, ch));
            }
        }
        self.mouse.clear_selection();
        self.redraw = true;
    }

    /// Type `text` into the active tab's split (opened first, in the main shell's
    /// folder), and press Enter if `enter`. The split gets the keys.
    pub(super) fn send_to_split(&mut self, text: String, enter: bool) -> Result<(), Error> {
        if self.split_holds_unsaved() {
            return Ok(());
        }
        if self.tabs[self.active]
            .split
            .as_ref()
            .is_some_and(|s| s.pty.is_none())
        {
            self.tabs[self.active].split = None; // a file pane makes room for the shell
        }
        if self.tabs[self.active].split.is_none() {
            let cwd = self.tabs[self.active].cwd();
            let shell = vec![self.shell()];
            let s =
                Session::spawn(&self.cfg, &shell, cwd.as_deref(), (80, 24)).map_err(Error::Pty)?;
            self.tabs[self.active].split = Some(Box::new(s));
            self.win.invalidate_buffers();
            self.regrid();
        }
        let tab = &mut self.tabs[self.active];
        if let Some(sp) = tab.split.as_mut() {
            sp.pending = Some((text, enter)); // typed once its shell shows a prompt
        }
        self.focus_pane(true);
        Ok(())
    }

    /// Close tab `i`'s split now (its shell ends); the main pane gets the width.
    pub(super) fn close_split(&mut self, i: usize) {
        if let Some(tab) = self.tabs.get_mut(i) {
            tab.split = None;
            tab.focus_split = false;
        }
        self.win.invalidate_buffers();
        self.regrid();
    }

    /// The × or Esc: close the active split, unless that would lose edits unasked.
    fn request_close_split(&mut self) {
        if !self.keep_unsaved(Some(self.active)) {
            self.close_split(self.active);
        }
    }

    /// Give the keys to the split (`true`) or the main pane.
    pub(super) fn focus_pane(&mut self, split: bool) {
        let focused = self.focused;
        let tab = &mut self.tabs[self.active];
        let split = split && tab.split.is_some();
        if tab.focus_split == split {
            return;
        }
        tab.pane_mut().focus(false);
        tab.focus_split = split;
        tab.pane_mut().focus(focused);
        self.mouse.clear_selection();
        self.redraw = true;
    }

    /// A press in the split header: ×, the Preview/Edit switch, or (anywhere else)
    /// focus. False if (x, y) is not on the header.
    pub(super) fn press_split_header(&mut self, (x, y): (f64, f64)) -> bool {
        let (Some(sx), top) = (self.split_x(), self.renderer.top as f64) else {
            return false;
        };
        if x < sx as f64 || y < top || y >= top + self.renderer.bar_height() as f64 {
            return false;
        }
        let toggle = self.split_doc().and_then(|d| d.toggle()).is_some();
        match self.renderer.header_hit(x as usize, self.win.width, toggle) {
            Hit::Close => self.request_close_split(),
            hit @ (Hit::Preview | Hit::Edit) => {
                let mode = if hit == Hit::Edit {
                    Mode::Edit
                } else {
                    Mode::Preview
                };
                if let Some(sp) = self.tabs[self.active].split.as_mut()
                    && let Some(doc) = sp.doc.as_mut()
                {
                    doc.set_mode(mode);
                    sp.refresh();
                }
                self.focus_pane(true);
                self.redraw = true;
            }
            Hit::Title => self.focus_pane(true),
        }
        true
    }

    fn split_doc(&self) -> Option<&crate::view::Doc> {
        self.tabs[self.active].split.as_ref()?.doc.as_ref()
    }
}

/// Files in the split.
impl App {
    /// Show `path` in the split at `line`. A shell that was there moves to a tab of
    /// its own; a file with unsaved edits stays (its header says why).
    pub(super) fn open_view(&mut self, path: &std::path::Path, line: usize) {
        let split = &mut self.tabs[self.active].split;
        if let Some(sp) = split.as_mut()
            && let Some(d) = sp.doc.as_mut().filter(|d| d.path == path)
        {
            // Already open: jump to the line, keep the edits.
            d.line = line.max(1);
            d.buf.go((d.line - 1, 0), false);
            sp.refresh();
            self.redraw = true;
            return self.focus_pane(true);
        }
        if self.split_holds_unsaved() {
            return;
        }
        let doc = match crate::view::Doc::open(path, line) {
            Ok(d) => d,
            Err(e) => return eprintln!("term: {}: {e}", path.display()),
        };
        if let Some(old) = self.tabs[self.active].split.take()
            && old.pty.is_some()
        {
            self.tabs.push(*old);
        }
        let s = Session::viewer(&self.cfg, doc, (80, 24));
        self.tabs[self.active].split = Some(Box::new(s));
        self.layout();
        self.win.invalidate_buffers();
        self.regrid();
        self.focus_pane(true);
    }

    /// The split holds a file with unsaved edits: say so in its header. True if so.
    fn split_holds_unsaved(&mut self) -> bool {
        let Some(doc) = self.tabs[self.active]
            .split
            .as_mut()
            .and_then(|s| s.doc.as_mut())
        else {
            return false;
        };
        if doc.buf.dirty() {
            doc.note = "unsaved: save (Ctrl+S) or close it first".into();
            self.redraw = true;
        }
        doc.buf.dirty()
    }

    /// Unsaved edits in tab `i` (or any tab, if None) stop a close the first time:
    /// the file's header says so and its tab is shown. Trying again goes ahead.
    pub(super) fn keep_unsaved(&mut self, which: Option<usize>) -> bool {
        let warn = |s: &mut Session| match s.doc.as_mut() {
            Some(d) if d.buf.dirty() && !d.confirm => {
                (d.confirm, d.note) = (true, "unsaved: close again to discard".into());
                true
            }
            _ => false,
        };
        let mut first = None;
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if which.is_none_or(|w| w == i) && (warn(t) | t.split.as_deref_mut().is_some_and(warn))
            {
                first = first.or(Some(i));
            }
        }
        if let Some(i) = first {
            super::input::log(self.select_tab(i));
            self.redraw = true;
        }
        first.is_some()
    }

    /// Tab `i`'s main shell ended. A shell in its split takes its place; a file
    /// with unsaved edits keeps the tab (a new shell starts beside it). Otherwise
    /// the tab closes. Returns false when that was the last tab.
    pub(super) fn main_exited(&mut self, i: usize) -> Result<bool, Error> {
        let tab = &mut self.tabs[i];
        match tab.split.take() {
            Some(sp) if sp.pty.is_some() => {
                *tab = Session {
                    name: tab.name.take(),
                    ..*sp
                }
            }
            Some(sp) if sp.doc.as_ref().is_some_and(|d| d.buf.dirty()) => {
                let shell = vec![self.shell()];
                let cwd = sp.cwd();
                let mut s = Session::spawn(&self.cfg, &shell, cwd.as_deref(), (80, 24))
                    .map_err(Error::Pty)?;
                (s.name, s.split, s.focus_split) = (self.tabs[i].name.take(), Some(sp), true);
                self.tabs[i] = s;
            }
            _ => return self.close_tab(i),
        }
        self.win.invalidate_buffers();
        self.layout();
        self.regrid();
        Ok(true)
    }

    /// A key in a file pane. False if the focused pane is a shell.
    pub(super) fn doc_key(&mut self, k: &Key) -> Result<bool, Error> {
        let pane = self.tabs[self.active].pane_mut();
        let rows = pane.term.rows();
        let Some(doc) = pane.doc.as_mut() else {
            return Ok(false);
        };
        match doc.key(k, rows) {
            Outcome::Done => pane.refresh(),
            Outcome::Scroll(n) => pane.term.scroll_view(n),
            Outcome::Copy(text) => {
                pane.refresh();
                self.win.copy(text)?;
            }
            Outcome::Close => self.request_close_split(),
        }
        self.redraw = true;
        Ok(true)
    }

    /// A left press or drag at `cell` of the focused pane, if it is a file being
    /// edited (places the cursor, selects). False otherwise.
    pub(super) fn doc_click(&mut self, cell: (usize, usize), drag: bool, double: bool) -> bool {
        let pane = self.tabs[self.active].pane_mut();
        let Some(doc) = pane.doc.as_mut().filter(|d| d.mode == Mode::Edit) else {
            return false;
        };
        let was = (doc.buf.cur, doc.buf.anchor);
        doc.click(cell, drag, double);
        if (doc.buf.cur, doc.buf.anchor) != was || !drag {
            pane.refresh();
            self.redraw = true;
        }
        true
    }

    /// Wheel over a file being edited: move its view. False otherwise.
    pub(super) fn doc_scroll(&mut self, lines: isize) -> bool {
        let pane = self.tabs[self.active].pane_mut();
        let Some(doc) = pane.doc.as_mut().filter(|d| d.mode == Mode::Edit) else {
            return false;
        };
        doc.scroll(lines);
        pane.refresh();
        self.redraw = true;
        true
    }
}

/// The divider.
impl App {
    /// (x, y) is on the divider: a few px either side of it, below the tab bar.
    pub(super) fn on_divider(&self, x: f64, y: f64) -> bool {
        let near = |sx: usize| (x - (sx as f64 - 0.5)).abs() <= GRAB;
        self.split_x().is_some_and(near) && y >= self.renderer.top as f64
    }

    /// The divider follows the pointer: panes repaint at once, grids refit after
    /// a short pause (as with a window resize).
    pub(super) fn drag_divider(&mut self, x: f64) {
        let old = self.split_x();
        self.split_frac = (x / self.win.width.max(1) as f64).clamp(0.0, 1.0) as f32;
        if self.split_x() != old {
            self.win.invalidate_buffers();
            self.regrid_at = Some(Instant::now() + REGRID_DELAY);
            self.redraw = true;
        }
    }
}
