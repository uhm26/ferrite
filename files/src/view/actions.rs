//! What can be done to the selected entry or the folder. The right-click menu and
//! the keys share this one list, so a key and its menu item never drift apart.
//! Long work goes to `ops::Jobs`; the Trash folder offers Restore and Delete Forever
//! (two steps: the first only asks).

use std::ffi::OsString;
use std::path::PathBuf;

use frame::ui::{MenuEvent, MenuItem, Rect, Ui};

use super::View;
use crate::browse::Browser;
use crate::ops::{self, Op};
use crate::{bookmarks, trash};

const MENU: &str = "menu";
const RENAME: &str = "rename";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Cut,
    Copy,
    Paste,
    Rename,
    Trash,
    Restore,
    Erase,
    NewFolder,
    CopyPath,
    Bookmark,
    Hidden,
}

/// Files picked with Ctrl+C / Ctrl+X, pasted with Ctrl+V.
pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

/// An open right-click menu: where, whether it is about the selected entry, and the
/// highlighted item.
pub struct Menu {
    pub at: (f32, f32),
    pub on_item: bool,
    pub hot: Option<usize>,
}

/// A rename in progress: the entry's real name and the text being edited.
pub struct Rename {
    pub from: OsString,
    pub text: String,
}

type Item = (Action, MenuItem<'static>);

fn item(a: Action, label: &'static str, hint: &'static str, on: bool) -> Item {
    (a, MenuItem::new(label, hint, on))
}

const SEP: Item = (Action::Open, MenuItem::SEPARATOR);

/// The menu for the selected entry (`on_item`) or for the folder itself.
fn items(b: &Browser, can_paste: bool, on_item: bool) -> Vec<Item> {
    let in_trash = b.cwd == trash::files_dir();
    let sel = b.entries.get(b.sel).filter(|_| on_item);
    let target = sel.map_or(b.cwd.clone(), |e| b.cwd.join(&e.os));
    let marked = bookmarks::has(&target);
    let mark = if marked {
        "Remove from Bookmarks"
    } else {
        "Add to Bookmarks"
    };
    let is_dir = sel.is_none_or(|e| e.dir);
    let mut v = Vec::new();
    if sel.is_some() {
        v.push(item(Action::Open, "Open", "Return", true));
        v.push(SEP);
        if in_trash {
            v.push(item(Action::Restore, "Restore", "", true));
            v.push(item(Action::Erase, "Delete Forever", "Delete", true));
        } else {
            v.push(item(Action::Cut, "Cut", "Ctrl+X", true));
            v.push(item(Action::Copy, "Copy", "Ctrl+C", true));
            v.push(item(Action::Paste, "Paste", "Ctrl+V", can_paste));
            v.push(SEP);
            v.push(item(Action::Rename, "Rename…", "F2", true));
            v.push(item(Action::Trash, "Move to Trash", "Delete", true));
        }
    } else {
        v.push(item(
            Action::NewFolder,
            "New Folder",
            "Ctrl+Shift+N",
            !in_trash,
        ));
        v.push(item(
            Action::Paste,
            "Paste",
            "Ctrl+V",
            can_paste && !in_trash,
        ));
        v.push(SEP);
        let hid = if b.hidden {
            "Hide Hidden Files"
        } else {
            "Show Hidden Files"
        };
        v.push(item(Action::Hidden, hid, "Ctrl+H", true));
    }
    v.push(SEP);
    v.push(item(Action::CopyPath, "Copy Path", "Ctrl+Shift+C", true));
    if is_dir {
        v.push(item(Action::Bookmark, mark, "Ctrl+D", true));
    }
    v
}

impl View {
    /// Open the menu at `at`: on the entry under the pointer (selecting it), else on
    /// the folder.
    pub(super) fn open_menu(
        &mut self,
        ui: &mut Ui,
        at: (f32, f32),
        on: Option<usize>,
        b: &mut Browser,
    ) {
        if let Some(i) = on {
            b.sel = i;
        }
        ui.forget(&format!("{MENU}/grow"));
        self.menu = Some(Menu {
            at,
            on_item: on.is_some(),
            hot: None,
        });
    }

    /// Draw the open menu (last, on top) and do what was picked.
    pub(super) fn menu(&mut self, ui: &mut Ui, b: &mut Browser) {
        let Some(m) = &mut self.menu else {
            return;
        };
        let list = items(b, self.clip.is_some(), m.on_item);
        let shown: Vec<MenuItem> = list.iter().map(|i| i.1).collect();
        match ui.menu(MENU, m.at, &shown, &mut m.hot) {
            MenuEvent::Open => {}
            MenuEvent::Closed => self.menu = None,
            MenuEvent::Picked(i) => {
                let on_item = m.on_item;
                self.menu = None;
                self.act(ui, b, list[i].0, on_item);
            }
        }
    }

    /// Do `a` to the selected entry (`on_item`) or to the folder.
    pub(super) fn act(&mut self, ui: &mut Ui, b: &mut Browser, a: Action, on_item: bool) {
        let sel = b
            .entries
            .get(b.sel)
            .map(|e| (b.cwd.join(&e.os), e.os.clone(), e.dir));
        let target = sel.as_ref().filter(|_| on_item);
        if a != Action::Erase {
            self.confirm = None;
        }
        match (a, target) {
            (Action::Open, Some(_)) => b.open(),
            (Action::Cut | Action::Copy, Some((p, ..))) => {
                self.clip = Some(Clip {
                    paths: vec![p.clone()],
                    cut: a == Action::Cut,
                });
                self.copy_text = Some(p.display().to_string());
                self.note = Some(format!(
                    "{} \u{201c}{}\u{201d}",
                    if a == Action::Cut { "Cut" } else { "Copied" },
                    name(p)
                ));
            }
            (Action::Paste, _) => self.paste_files(b),
            (Action::Rename, Some((_, os, dir))) => self.start_rename(ui, os.clone(), *dir),
            (Action::Trash, Some((p, ..))) => self.run(Op::Trash(vec![p.clone()])),
            (Action::Restore, Some((_, os, _))) => {
                self.run(Op::Restore(vec![os.to_string_lossy().into_owned()]))
            }
            (Action::Erase, Some((_, os, _))) => self.erase(os.clone()),
            (Action::NewFolder, _) => self.new_folder(ui, b),
            (Action::CopyPath, t) => {
                let p = t.map_or(b.cwd.clone(), |t| t.0.clone());
                self.note = Some(format!("Copied path {}", p.display()));
                self.copy_text = Some(p.display().to_string());
            }
            (Action::Bookmark, t) => self.bookmark(t.map_or(b.cwd.clone(), |t| t.0.clone())),
            (Action::Hidden, _) => b.toggle_hidden(),
            _ => {}
        }
    }

    fn run(&mut self, op: Op) {
        if let Err(e) = self.jobs.start(op) {
            self.note = Some(format!("Error: {e}"));
        }
    }

    fn paste_files(&mut self, b: &Browser) {
        let Some(c) = self.clip.take() else {
            return;
        };
        // A copy can be pasted again; a cut moves once.
        let op = if c.cut {
            Op::Move(c.paths, b.cwd.clone())
        } else {
            let op = Op::Copy(c.paths.clone(), b.cwd.clone());
            self.clip = Some(c);
            op
        };
        self.run(op);
    }

    /// Delete Forever asks first: the same action again on the same item does it.
    fn erase(&mut self, name: OsString) {
        if self.confirm.as_ref() == Some(&name) {
            self.confirm = None;
            self.run(Op::Erase(vec![name.to_string_lossy().into_owned()]));
        } else {
            self.note = Some(format!(
                "Delete \u{201c}{}\u{201d} forever? Press Delete again",
                name.to_string_lossy()
            ));
            self.confirm = Some(name);
        }
    }

    fn new_folder(&mut self, ui: &mut Ui, b: &mut Browser) {
        match ops::new_folder(&b.cwd) {
            Ok(name) => {
                b.reload_select(&name);
                self.start_rename(ui, name, true);
            }
            Err(e) => self.note = Some(format!("Error: {e}")),
        }
    }

    fn bookmark(&mut self, p: PathBuf) {
        self.note = Some(match bookmarks::toggle(&p) {
            Ok(true) => format!("Bookmarked \u{201c}{}\u{201d}", name(&p)),
            Ok(false) => format!("Removed bookmark \u{201c}{}\u{201d}", name(&p)),
            Err(e) => format!("Error: {e}"),
        });
        self.places.refresh();
    }

    /// Edit the name in place, the part before the extension selected (like Nemo).
    fn start_rename(&mut self, ui: &mut Ui, from: OsString, dir: bool) {
        let text = from.to_string_lossy().into_owned();
        let end = text
            .rfind('.')
            .filter(|&i| i > 0 && !dir)
            .unwrap_or(text.len());
        ui.take_focus_range(RENAME, 0, end);
        self.rename = Some(Rename { from, text });
    }

    /// Draw the rename field over the selected entry's name (`at`); Return renames,
    /// Esc or a click elsewhere cancels.
    pub(super) fn rename(&mut self, ui: &mut Ui, b: &mut Browser, at: Option<Rect>) {
        let (Some(r), Some(at)) = (&mut self.rename, at) else {
            self.rename = None;
            return;
        };
        ui.field(RENAME, at, &mut r.text, "");
        if ui.submitted(RENAME) {
            match ops::rename(&b.cwd, &r.from, &r.text) {
                Ok(()) => b.reload_select(OsString::from(r.text.trim()).as_os_str()),
                Err(e) => self.note = Some(format!("Error: {e}")),
            }
            self.rename = None;
        } else if !ui.focused(RENAME) {
            self.rename = None;
        }
    }

    /// Finished jobs: show their reports, read the folder again.
    pub fn job_reports(&mut self, b: &mut Browser) {
        let reports = self.jobs.reports();
        if let Some(last) = reports.last() {
            self.note = Some(last.clone());
            b.reload();
            self.places.refresh();
        }
    }
}

fn name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
