//! Tabs: several folders open at once, each its own [`Browser`] (history, selection,
//! filter). One is shown; the others wait. A new tab takes the shown one's view
//! settings (hidden files, sort).

use std::path::PathBuf;

use crate::browse::Browser;

pub struct Tabs {
    list: Vec<Browser>,
    /// A stable id per tab, for its animations (index shifts when one closes).
    ids: Vec<u64>,
    next_id: u64,
    cur: usize,
    /// Split view: the tab shown beside the current one (by id); it sits on the side
    /// of the strip it is on.
    split: Option<u64>,
}

impl Tabs {
    pub fn new(first: Browser) -> Self {
        Tabs {
            list: vec![first],
            ids: vec![0],
            next_id: 1,
            cur: 0,
            split: None,
        }
    }

    pub fn cur(&self) -> &Browser {
        &self.list[self.cur]
    }

    pub fn cur_mut(&mut self) -> &mut Browser {
        &mut self.list[self.cur]
    }

    pub fn index(&self) -> usize {
        self.cur
    }

    /// Each tab's id and folder, in order.
    pub fn each(&self) -> impl Iterator<Item = (u64, &PathBuf)> {
        self.ids
            .iter()
            .copied()
            .zip(self.list.iter().map(|b| &b.cwd))
    }

    /// Open `path` in a new tab right after the shown one; `show` switches to it.
    pub fn open(&mut self, path: PathBuf, show: bool) {
        let b = Browser::new_like(path, self.cur());
        let at = self.cur + 1;
        self.list.insert(at, b);
        self.ids.insert(at, self.next_id);
        self.next_id += 1;
        if show {
            self.cur = at;
        }
    }

    /// Close tab `i`. False if it was the last one (nothing closes then: the window
    /// should).
    pub fn close(&mut self, i: usize) -> bool {
        if self.list.len() <= 1 || i >= self.list.len() {
            return false;
        }
        if i == self.cur || Some(self.ids[i]) == self.split {
            self.split = None;
        }
        self.list.remove(i);
        self.ids.remove(i);
        if self.cur > i || self.cur == self.list.len() {
            self.cur -= 1;
        }
        self.cur_mut().reload();
        true
    }

    /// Show tab `i`, read again (its folder may have changed while hidden).
    pub fn show(&mut self, i: usize) {
        if Some(i) == self.other() {
            self.focus_other();
        } else if i < self.list.len() && i != self.cur {
            self.cur = i;
            self.cur_mut().reload();
        }
    }

    /// The tab shown beside the current one (split view), if any.
    pub fn other(&self) -> Option<usize> {
        self.ids.iter().position(|&id| Some(id) == self.split)
    }

    /// Tab `i`'s folder.
    pub fn folder(&self, i: usize) -> PathBuf {
        self.list[i].cwd.clone()
    }

    /// Ids of the current tab and of the one beside it.
    pub fn pair_ids(&self) -> (u64, Option<u64>) {
        (self.ids[self.cur], self.split)
    }

    /// The other pane sits on the left of the current one.
    pub fn other_left(&self) -> bool {
        self.other().is_some_and(|o| o < self.cur)
    }

    /// The current tab, and the one beside it in split view.
    pub fn pair_mut(&mut self) -> (&mut Browser, Option<&mut Browser>) {
        let Some(o) = self.other() else {
            return (&mut self.list[self.cur], None);
        };
        let (lo, hi) = (o.min(self.cur), o.max(self.cur));
        let (a, b) = self.list.split_at_mut(hi);
        let (lo, hi) = (&mut a[lo], &mut b[0]);
        if self.cur < o {
            (lo, Some(hi))
        } else {
            (hi, Some(lo))
        }
    }

    /// A middle click on tab `i`: show it beside the current one (the tab there goes
    /// back to the strip). On the tab already beside: back to one pane.
    pub fn split_with(&mut self, i: usize) {
        if i >= self.list.len() || i == self.cur {
            return;
        }
        let id = self.ids[i];
        self.split = if self.split == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    /// Back to one pane (F3).
    pub fn unsplit(&mut self) {
        self.split = None;
    }

    /// Tab `i`'s browser.
    pub fn get(&self, i: usize) -> &Browser {
        &self.list[i]
    }

    /// Drag tab `from` to place `to`; the current tab stays current.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        let n = self.list.len();
        if from == to || from >= n || to >= n {
            return;
        }
        let cur_id = self.ids[self.cur];
        let b = self.list.remove(from);
        let id = self.ids.remove(from);
        self.list.insert(to, b);
        self.ids.insert(to, id);
        self.cur = self.ids.iter().position(|&i| i == cur_id).unwrap_or(0);
    }

    /// Make the other pane the current one (a click in it, Tab).
    pub fn focus_other(&mut self) {
        if let Some(o) = self.other() {
            self.split = Some(self.ids[self.cur]);
            self.cur = o;
        }
    }

    /// Read every tab's folder again (something changed on disk).
    pub fn reload_all(&mut self) {
        for b in &mut self.list {
            b.reload();
        }
    }

    /// The next (`d` = 1) or previous (-1) tab, wrapping around.
    pub fn step(&mut self, d: isize) {
        let n = self.list.len() as isize;
        self.show((self.cur as isize + d).rem_euclid(n) as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn open_close_and_step_keep_the_right_tab_shown() {
        let root = std::env::temp_dir().join(format!("files-tabs-{}", std::process::id()));
        for d in ["a", "b", "c"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut t = Tabs::new(Browser::new(root.join("a")));
        t.cur_mut().toggle_hidden();
        t.open(root.join("b"), false);
        let behind = (t.index(), t.list.len());
        t.open(root.join("c"), true);
        let order: Vec<PathBuf> = t.each().map(|(_, p)| p.clone()).collect();
        let like = t.cur().hidden;
        t.step(1);
        let wrapped = t.cur().cwd.clone();
        t.show(1);
        assert!(t.close(0));
        let after_close = (t.index(), t.cur().cwd.clone());
        assert!(t.close(1));
        let last = t.close(0);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(behind, (0, 2), "a background tab doesn't switch");
        assert_eq!(order, [root.join("a"), root.join("c"), root.join("b")]);
        assert!(like, "a new tab takes the hidden-files setting");
        assert_eq!(wrapped, root.join("b"));
        assert_eq!(after_close, (0, root.join("c")));
        assert!(!last, "the last tab stays");
    }

    #[test]
    fn split_pairs_two_tabs_and_focus_swaps_sides() {
        let root = std::env::temp_dir().join(format!("files-split-{}", std::process::id()));
        fs::create_dir_all(root.join("x")).unwrap();
        let mut t = Tabs::new(Browser::new(root.clone()));
        t.open(root.clone(), false);
        t.open(root.clone(), false);
        t.split_with(2);
        let opened = (t.other(), t.other_left());
        t.split_with(1);
        let replaced = t.other();
        let (a, b) = t.pair_mut();
        b.unwrap().go(root.join("x"), None);
        let left_cwd = a.cwd.clone();
        t.focus_other();
        let focused = (t.index(), t.other(), t.other_left(), t.cur().cwd.clone());
        t.move_tab(1, 2);
        let moved = (t.index(), t.other(), t.cur().cwd.clone());
        t.split_with(0);
        let off = t.other();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(opened, (Some(2), false), "middle-clicked tab goes beside");
        assert_eq!(replaced, Some(1), "another middle click replaces it");
        assert_eq!(left_cwd, root);
        assert_eq!(focused, (1, Some(0), true, root.join("x")));
        assert_eq!(
            moved,
            (2, Some(0), root.join("x")),
            "current follows its tab"
        );
        assert_eq!(off, None, "middle click on the tab beside: one pane again");
    }
}
