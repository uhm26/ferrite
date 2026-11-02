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

impl App {
    /// The left button is held on a tab: moving drags it.
    pub(super) fn dragging_tab(&self) -> bool {
        self.tab_press == Some(BTN_LEFT)
    }

    /// A press on the bar. Returns false when the last tab was closed.
    pub(super) fn press_tab(&mut self, button: u32) -> Result<bool, Error> {
        // Remembered so its release goes nowhere (not to an app with mouse reporting).
        self.tab_press = Some(button);
        self.finish_rename()?;
        let Some(i) = self
            .renderer
            .tab_at(self.mouse.x, self.win.width, &self.labels)
        else {
            return Ok(true);
        };
        match button {
            BTN_LEFT => self.select_tab(i)?,
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

    /// While a tab is held, it swaps with a neighbour once the pointer passes that
    /// neighbour's middle. (Swapping as soon as the pointer is over it flips a short
    /// tab back and forth next to a long one: after the swap the pointer is over the
    /// long one again.) Loops so a fast drag can pass several tabs.
    pub(super) fn drag_tab(&mut self) {
        loop {
            let spans = self.renderer.tab_spans(&self.labels, self.win.width);
            let Some(to) = drag_target(&spans, self.active, self.mouse.x) else {
                return;
            };
            self.tabs.swap(self.active, to);
            self.active = to;
            self.refresh_labels();
            self.redraw = true;
        }
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

/// Where the held tab `i` goes for pointer `x`: a neighbour whose middle the pointer
/// has passed, else nowhere. `spans` are (x, width) per tab.
fn drag_target(spans: &[(usize, usize)], i: usize, x: f64) -> Option<usize> {
    let mid = |j: usize| spans.get(j).map(|&(x, w)| (x + w / 2) as f64);
    match (mid(i + 1), i.checked_sub(1).and_then(mid)) {
        (Some(m), _) if x > m => Some(i + 1),
        (_, Some(m)) if x < m => Some(i - 1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::drag_target;

    #[test]
    fn short_tab_next_to_long_one_does_not_flip() {
        // Short tab (50 px) dragged right over a long one (200 px).
        let before = [(0, 50), (50, 200)];
        assert_eq!(drag_target(&before, 0, 120.0), None); // not past the middle (150)
        assert_eq!(drag_target(&before, 0, 160.0), Some(1));
        // After the swap: long tab first, held short tab second, pointer unchanged.
        let after = [(0, 200), (200, 50)];
        assert_eq!(drag_target(&after, 1, 160.0), None); // stays put: no flip back
        assert_eq!(drag_target(&after, 1, 90.0), Some(0)); // back left past its middle
    }
}
