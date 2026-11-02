//! Tabs: open (in the home folder), close, switch, and the layout change
//! when the bar appears or goes away (it shows with two or more tabs).

use super::session::Session;
use super::{App, Error};

impl App {
    pub(super) fn open_tab(&mut self) -> Result<(), Error> {
        let home = crate::pty::home();
        let shell = vec![self.shell()];
        let size = (
            self.tabs[self.active].term.cols(),
            self.tabs[self.active].term.rows(),
        );
        let s = Session::spawn(&self.cfg, &shell, Some(&home), size).map_err(Error::Pty)?;
        self.tabs.push(s);
        self.select_tab(self.tabs.len() - 1)?;
        self.layout();
        Ok(())
    }

    /// Close tab `i`. Returns false when it was the last one.
    pub(super) fn close_tab(&mut self, i: usize) -> Result<bool, Error> {
        if i >= self.tabs.len() || self.keep_unsaved(Some(i)) {
            return Ok(true);
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            return Ok(false);
        }
        let next = if self.active > i || self.active >= self.tabs.len() {
            self.active.saturating_sub(1)
        } else {
            self.active
        };
        self.active = usize::MAX; // force the switch below to run
        self.select_tab(next)?;
        self.layout();
        Ok(true)
    }

    pub(super) fn select_tab(&mut self, i: usize) -> Result<(), Error> {
        if i >= self.tabs.len() || i == self.active {
            return Ok(());
        }
        if let Some(old) = self.tabs.get_mut(self.active) {
            old.focus(false);
        }
        self.active = i;
        let focused = self.focused;
        self.tabs[i].focus(focused);
        self.tabs[i].term.dirty = true;
        self.mouse.clear_selection();
        self.redraw = true;
        self.update_title()
    }

    /// Show or hide the bar for the current tab count and refit every tab.
    pub(super) fn layout(&mut self) {
        self.refresh_labels();
        let top = if self.tabs.len() > 1 {
            self.renderer.bar_height()
        } else {
            0
        };
        if top != self.renderer.top {
            self.renderer.top = top;
            self.win.invalidate_buffers();
            let (w, h) = (self.win.width, self.win.height);
            self.resize(w, h);
            self.regrid();
        }
    }

    pub(super) fn update_title(&mut self) -> Result<(), Error> {
        let title = self.tabs[self.active].title();
        if title != self.title {
            self.win.set_title(&title)?;
            self.title = title;
        }
        Ok(())
    }

    /// Labels for the bar, refreshed only when a title changes.
    pub(super) fn refresh_labels(&mut self) {
        self.labels.clear();
        self.labels.extend(self.tabs.iter().map(Session::label));
    }
}
