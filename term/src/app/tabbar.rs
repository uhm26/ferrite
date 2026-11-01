//! Tab bar by mouse: left click switches, drag moves a tab, right click renames,
//! middle click closes. Renaming starts empty: type, Backspace, Enter saves, Esc
//! cancels, a click elsewhere saves. Saving it empty goes back to the shell's title.

use super::{App, Error};
use crate::wl::proto::pointer::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};
use crate::xkb::Key;

// Keysyms (xkbcommon-keysyms.h).
const BACKSPACE: u32 = 0xff08;
const RETURN: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;
const KP_ENTER: u32 = 0xff8d;
const MAX_NAME: usize = 64;

/// A left press on a tab, held until release (dragging moves the tab).
#[derive(Clone, Copy)]
pub(super) struct TabPress;

impl App {
    pub(super) fn dragging_tab(&self) -> bool {
        self.tab_press.is_some()
    }

    /// The tab slot under the pointer, past the last tab = the last one.
    fn slot(&self) -> usize {
        let n = self.tabs.len();
        let w = self.win.width;
        self.renderer
            .tab_at(self.mouse.x, w, &self.labels)
            .unwrap_or(n - 1)
    }

    /// A press on the bar. Returns false when the last tab was closed.
    pub(super) fn press_tab(&mut self, button: u32) -> Result<bool, Error> {
        self.finish_rename()?;
        let Some(i) = self
            .renderer
            .tab_at(self.mouse.x, self.win.width, &self.labels)
        else {
            return Ok(true);
        };
        match button {
            BTN_LEFT => {
                self.tab_press = Some(TabPress);
                self.select_tab(i)?;
            }
            BTN_RIGHT => {
                self.select_tab(i)?;
                self.tabs[i].editing = Some(String::new());
                self.refresh_labels();
                self.redraw = true;
            }
            BTN_MIDDLE => return self.close_tab(i),
            _ => {}
        }
        Ok(true)
    }

    /// While a tab is held, moving over another slot moves the tab there.
    pub(super) fn drag_tab(&mut self) {
        let to = self.slot();
        if to == self.active {
            return;
        }
        let tab = self.tabs.remove(self.active);
        self.tabs.insert(to, tab);
        self.active = to;
        self.refresh_labels();
        self.redraw = true;
    }

    pub(super) fn release_tab(&mut self) {
        self.tab_press = None;
    }

    /// The tab being renamed, if any.
    pub(super) fn renaming(&self) -> Option<usize> {
        self.tabs.iter().position(|s| s.editing.is_some())
    }

    /// A key while renaming tab `i`; the shell never sees it.
    pub(super) fn rename_key(&mut self, i: usize, k: &Key) -> Result<(), Error> {
        let s = &mut self.tabs[i];
        let Some(text) = &mut s.editing else {
            return Ok(());
        };
        match k.sym {
            RETURN | KP_ENTER => return self.finish_rename(),
            ESCAPE => s.editing = None,
            BACKSPACE => drop(text.pop()),
            _ if !k.mods.ctrl && !k.mods.alt => {
                for c in k.text.chars().filter(|c| !c.is_control()) {
                    if text.chars().count() < MAX_NAME {
                        text.push(c);
                    }
                }
            }
            _ => {}
        }
        self.refresh_labels();
        self.redraw = true;
        Ok(())
    }

    /// Save a rename in progress, if there is one.
    pub(super) fn finish_rename(&mut self) -> Result<(), Error> {
        let Some(i) = self.renaming() else {
            return Ok(());
        };
        let s = &mut self.tabs[i];
        let text = s.editing.take().unwrap_or_default();
        let text = text.trim();
        s.name = (!text.is_empty()).then(|| text.to_string());
        self.refresh_labels();
        self.redraw = true;
        self.update_title()
    }
}
