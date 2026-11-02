//! What can be done to the selected entry or the folder. The right-click menu and
//! the keys share this one list, so a key and its menu item never drift apart.
//! Long work goes to `ops::Jobs`; the Trash folder offers Restore and Delete Forever
//! (two steps: the first only asks).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use frame::ui::theme::*;
use frame::ui::{Align, MenuEvent, MenuHot, MenuItem, Rect, Ui};

use super::View;
use super::menu::{NEW_FILES, OpenWith, items, shown};
use super::tabs::TabReq;
use crate::apps;
use crate::bookmarks;
use crate::browse::Browser;
use crate::ops::archive;
use crate::ops::copy::Conflict;
use crate::ops::{Op, Report, Undo, create};

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
    /// A terminal in the folder (or the selected folder).
    Terminal,
    Extract,
    /// Index into `archive::FORMATS`.
    Compress(usize),
    /// Index into the Open With apps; Ctrl also makes it the default.
    OpenWith(usize),
    /// Open the selection with a typed command.
    OpenCommand,
    /// Show or hide the info pane.
    Info,
    /// Index into `NEW_FILES`, then into the templates.
    NewFile(usize),
    EmptyTrash,
    /// A link to each selected entry, in this folder.
    MakeLink,
    /// Index into the Open With apps: the default for this type from now on.
    SetDefault(usize),
    /// The selected folders, each in a tab behind this one.
    NewTab,
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
    pub hot: MenuHot,
}

/// A copy or move waiting for an answer about names already taken.
pub struct Transfer {
    pub paths: Vec<PathBuf>,
    pub dir: PathBuf,
    pub copy: bool,
}

/// A rename in progress: the entry's real name and the text being edited.
pub struct Rename {
    pub from: OsString,
    pub text: String,
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
        // On a selected entry the whole selection stays; on another, it alone is selected.
        match on {
            Some(i) if b.is_selected(i) => b.sel = i,
            Some(i) => b.click(i, (false, false)),
            None => {}
        }
        self.templates = create::templates();
        // Apps for the entry's type, looked up once per menu (xdg-mime is a process).
        let file = on
            .and_then(|i| b.entries.get(i))
            .filter(|e| !e.dir)
            .map(|e| b.cwd.join(&e.os));
        self.open_with = file.and_then(|f| apps::mime_of(&f)).map(|mime| {
            let (apps, has_default) = apps::for_mime(&mime);
            OpenWith {
                mime,
                apps,
                has_default,
            }
        });
        ui.forget(&format!("{MENU}/grow"));
        self.menu = Some(Menu {
            at,
            on_item: on.is_some(),
            hot: MenuHot::default(),
        });
    }

    /// Draw the open menu (last, on top) and do what was picked.
    pub(super) fn menu(&mut self, ui: &mut Ui, b: &mut Browser) {
        let Some(m) = &mut self.menu else {
            return;
        };
        let extras = (self.templates.as_slice(), self.open_with.as_ref());
        let list = items(b, (self.clip.is_some(), m.on_item), extras);
        let subs: Vec<Vec<MenuItem>> = list
            .iter()
            .map(|e| e.sub.iter().map(|s| shown(s, &[])).collect())
            .collect();
        let top: Vec<MenuItem> = list.iter().zip(&subs).map(|(e, s)| shown(e, s)).collect();
        match ui.menu(MENU, m.at, &top, &mut m.hot) {
            MenuEvent::Open => {}
            MenuEvent::Closed => self.menu = None,
            MenuEvent::Picked(i, j) => {
                // The menu shrinks away by itself; it goes on `Closed`.
                let on_item = m.on_item;
                let a = j.map_or(list[i].action, |j| list[i].sub[j].action);
                self.act(ui, b, a, on_item);
                ui.redraw(); // what it changed shows now, not on the next event
            }
        }
    }

    /// Do `a` to the selection (`on_item`: Cut, Copy, Trash … take every selected
    /// entry; Open and Rename the cursor's) or to the folder.
    pub(super) fn act(&mut self, ui: &mut Ui, b: &mut Browser, a: Action, on_item: bool) {
        if !matches!(a, Action::Erase | Action::EmptyTrash) {
            self.confirm = None;
        }
        let paths = if on_item {
            b.selected_paths()
        } else {
            Vec::new()
        };
        let cursor = b
            .entries
            .get(b.sel)
            .filter(|_| on_item)
            .map(|e| (e.os.clone(), e.dir));
        let some = !paths.is_empty();
        match a {
            Action::Open if cursor.is_some() => self.open_entry(b),
            Action::Cut | Action::Copy if some => self.clip_files(paths, a == Action::Cut),
            Action::Paste => self.paste_files(b),
            Action::Rename if b.search.is_none() => self.rename_action(ui, paths, cursor),
            Action::MakeLink if some => self.make_links(&paths, &b.cwd),
            Action::SetDefault(i) => self.set_default(i),
            Action::Trash if some => self.run(Op::Trash(paths)),
            Action::Restore if some => self.run(Op::Restore(names(&paths))),
            Action::Erase if some => self.erase(names(&paths)),
            Action::NewFolder => self.new_folder(ui, b),
            Action::NewFile(i) => self.new_file(ui, b, i),
            Action::EmptyTrash => self.empty_trash(false),
            Action::CopyPath => {
                let list = if some { paths } else { vec![b.cwd.clone()] };
                let text: Vec<String> = list.iter().map(|p| p.display().to_string()).collect();
                self.note = Some(format!("Copied {}", count_or_name("path", &list)));
                self.copy_text = Some(text.join("\n"));
            }
            Action::Bookmark => {
                self.bookmark(paths.first().cloned().unwrap_or_else(|| b.cwd.clone()))
            }
            Action::Hidden => b.toggle_hidden(),
            Action::Info => self.info_open = !self.info_open,
            Action::Terminal => {
                let into = cursor.filter(|c| c.1).map(|c| b.cwd.join(c.0));
                self.terminal(into.unwrap_or_else(|| b.cwd.clone()));
            }
            Action::Extract => {
                if let Some(p) = paths.first() {
                    self.run(Op::Extract(p.clone(), b.cwd.clone()));
                }
            }
            Action::Compress(i) if some => self.compress(b, paths, i),
            Action::OpenWith(i) if some => self.open_with(ui, paths, i),
            Action::OpenCommand if some => self.loc.start_command(ui, paths),
            Action::NewTab => {
                let dirs = paths.into_iter().filter(|p| p.is_dir());
                self.tab_reqs.extend(dirs.map(|p| TabReq::Open(p, false)));
            }
            _ => {}
        }
    }

    /// F2: one entry gets the name field on it; several get new names at once.
    fn rename_action(
        &mut self,
        ui: &mut Ui,
        paths: Vec<PathBuf>,
        cursor: Option<(OsString, bool)>,
    ) {
        if paths.len() > 1 {
            self.loc.start_rename(ui, paths);
        } else if let Some((os, dir)) = cursor {
            self.start_rename(ui, os, dir);
        }
    }

    /// Ctrl+C / Ctrl+X: remember the files, and put their paths on the clipboard too.
    fn clip_files(&mut self, paths: Vec<PathBuf>, cut: bool) {
        let what = count_or_name("item", &paths);
        self.note = Some(format!("{} {what}", if cut { "Cut" } else { "Copied" }));
        let text: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        self.copy_text = Some(text.join("\n"));
        self.clip = Some(Clip { paths, cut });
    }

    /// Open `paths` with Open With app `i`; with Ctrl held it becomes the default too.
    fn open_with(&mut self, ui: &Ui, paths: Vec<PathBuf>, i: usize) {
        let Some(w) = &self.open_with else {
            return;
        };
        let Some(app) = w.apps.get(i) else {
            return;
        };
        let files: Vec<PathBuf> = paths.into_iter().filter(|p| !p.is_dir()).collect();
        if ui.s.input.mods.ctrl {
            self.note = Some(match apps::set_default(app, &w.mime) {
                Ok(()) => format!("{} opens {} from now on", app.name, w.mime),
                Err(e) => format!("Error: {e}"),
            });
        }
        if let Err(e) = apps::launch(app, &files) {
            self.note = Some(format!("Error: {}: {e}", app.name));
        }
    }

    /// Make the menu's app `i` the default for the file's type (no opening).
    fn set_default(&mut self, i: usize) {
        let Some(w) = &self.open_with else {
            return;
        };
        let Some(app) = w.apps.get(i) else {
            return;
        };
        self.note = Some(match apps::set_default(app, &w.mime) {
            Ok(()) => format!("{} opens {} from now on", app.name, w.mime),
            Err(e) => format!("Error: {e}"),
        });
    }

    /// "Link to …" for each of `paths`, in `dir` (Ctrl+Z removes them).
    fn make_links(&mut self, paths: &[PathBuf], dir: &Path) {
        let mut made = Vec::new();
        for p in paths {
            match create::link(p, dir) {
                Ok(at) => made.push(at),
                Err(e) => self.note = Some(format!("Error: {}: {e}", p.display())),
            }
        }
        if !made.is_empty() {
            self.note = Some(format!("Made {}", count_or_name("link", &made)));
            self.undo.push(Undo::Made(made));
        }
    }

    /// Open `$TERMINAL` (else our `term`) in `dir`.
    fn terminal(&mut self, dir: PathBuf) {
        let term = std::env::var_os("TERMINAL").unwrap_or_else(|| "term".into());
        match std::process::Command::new(&term).current_dir(&dir).spawn() {
            // Reap it off-thread so no zombie stays behind.
            Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
            Err(e) => self.note = Some(format!("Error: {}: {e}", term.to_string_lossy())),
        }
    }

    /// Pack the selection into "<name>.zip" (or "Archive.zip" for several), numbered
    /// if taken, in format `archive::FORMATS[i]`.
    fn compress(&mut self, b: &Browser, paths: Vec<PathBuf>, i: usize) {
        let stem = match paths.as_slice() {
            [p] => name(p),
            _ => "Archive".to_string(),
        };
        let out = b
            .cwd
            .join(create::free_numbered(&b.cwd, &stem, archive::FORMATS[i]));
        self.run(Op::Compress(paths, out));
    }

    pub(super) fn run(&mut self, op: Op) {
        if let Err(e) = self.jobs.start(op) {
            self.note = Some(format!("Error: {e}"));
        }
    }

    /// Ctrl+V: copy or move the clipped files here, asking first if names clash.
    fn paste_files(&mut self, b: &Browser) {
        let Some(c) = self.clip.take() else {
            return;
        };
        // A copy can be pasted again; a cut moves once.
        if !c.cut {
            self.clip = Some(Clip {
                paths: c.paths.clone(),
                cut: false,
            });
        }
        self.transfer(c.paths, b.cwd.clone(), !c.cut);
    }

    /// Copy (`copy`) or move `paths` into `dir`; names already taken there ask
    /// Keep Both / Replace / Skip first (paste and drop both come here).
    pub(super) fn transfer(&mut self, paths: Vec<PathBuf>, dir: PathBuf, copy: bool) {
        let clash = crate::ops::copy::clashes(&paths, &dir, !copy);
        if clash == 0 {
            self.finish_transfer(Transfer { paths, dir, copy }, Conflict::KeepBoth);
        } else {
            self.prompt = Some((Transfer { paths, dir, copy }, clash));
        }
    }

    pub(super) fn finish_transfer(&mut self, t: Transfer, c: Conflict) {
        let op = if t.copy {
            Op::Copy(t.paths, t.dir, c)
        } else {
            Op::Move(t.paths, t.dir, c)
        };
        self.run(op);
    }

    /// Delete Forever asks first: the same action again on the same items does it.
    fn erase(&mut self, names: Vec<String>) {
        // No name holds "/": joined, they key this one question.
        let key = OsString::from(names.join("/"));
        if self.confirm.as_ref() == Some(&key) {
            self.confirm = None;
            self.run(Op::Erase(names));
        } else {
            let what = if names.len() == 1 {
                format!("\u{201c}{}\u{201d}", names[0])
            } else {
                format!("{} items", names.len())
            };
            self.note = Some(format!("Delete {what} forever? Press Delete again"));
            self.confirm = Some(key);
        }
    }

    fn new_folder(&mut self, ui: &mut Ui, b: &mut Browser) {
        match create::new_folder(&b.cwd) {
            Ok(name) => {
                self.undo.push(Undo::Made(vec![b.cwd.join(&name)]));
                b.reload_select(&name);
                self.start_rename(ui, name, true);
            }
            Err(e) => self.note = Some(format!("Error: {e}")),
        }
    }

    fn new_file(&mut self, ui: &mut Ui, b: &mut Browser, i: usize) {
        let made = match NEW_FILES.get(i) {
            Some(&(_, stem, ext, content, exec)) => {
                create::new_file(&b.cwd, (stem, ext), content.as_bytes(), exec)
            }
            None => match self.templates.get(i - NEW_FILES.len()) {
                Some(t) => create::from_template(&b.cwd, t),
                None => return,
            },
        };
        match made {
            Ok(name) => {
                self.undo.push(Undo::Made(vec![b.cwd.join(&name)]));
                b.reload_select(&name);
                self.start_rename(ui, name, false);
            }
            Err(e) => self.note = Some(format!("Error: {e}")),
        }
    }

    /// Empty the Trash. From the menu it asks first (`held` false: the same pick again
    /// does it); the hold-to-fill button already was the confirmation.
    pub(super) fn empty_trash(&mut self, held: bool) {
        let ask = OsString::from("/"); // no file is called "/": marks this question
        if held || self.confirm.as_ref() == Some(&ask) {
            self.confirm = None;
            self.run(Op::EmptyTrash);
        } else {
            self.note = Some("Empty the Trash for good? Pick Empty Trash again".into());
            self.confirm = Some(ask);
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
            match create::rename(&b.cwd, &r.from, &r.text) {
                Ok(()) => {
                    let new = r.text.trim();
                    self.undo
                        .push(Undo::Renamed(b.cwd.join(&r.from), b.cwd.join(new)));
                    b.reload_select(OsString::from(new).as_os_str());
                }
                Err(e) => self.note = Some(format!("Error: {e}")),
            }
            self.rename = None;
        } else if !ui.focused(RENAME) {
            self.rename = None;
        }
    }

    /// Finished jobs: show their reports, read the folder again.
    pub fn job_reports(&mut self, b: &mut Browser) {
        let mut changed = false;
        for r in self.jobs.reports() {
            match r {
                Report::Done { line, undo } => {
                    self.note = Some(line);
                    self.undo.extend(undo);
                    changed = true;
                }
                Report::Size(p, bytes, files) => {
                    self.measuring.remove(&p);
                    self.sizes.insert(p, (bytes, files));
                }
            }
        }
        if changed {
            // Files changed: folder sizes are stale.
            self.sizes.clear();
            b.reload();
            self.places.refresh();
        }
    }

    /// Ctrl+Z: take the last finished operation back.
    pub(super) fn undo_last(&mut self, b: &mut Browser) {
        let Some(u) = self.undo.pop() else {
            self.note = Some("Nothing to undo".into());
            return;
        };
        match u {
            Undo::Moved(pairs) => self.run(Op::MoveBack(pairs)),
            Undo::Made(paths) => self.run(Op::Trash(paths)),
            Undo::Trashed(names) => self.run(Op::Restore(names)),
            Undo::Renamed(old, new) => match std::fs::rename(&new, &old) {
                Ok(()) => {
                    self.note = Some(format!(
                        "Undone: renamed back to \u{201c}{}\u{201d}",
                        name(&old)
                    ));
                    if let Some(n) = old.file_name() {
                        b.reload_select(n);
                    }
                }
                Err(e) => self.note = Some(format!("Error: {e}")),
            },
        }
    }
}

fn name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// File names of `paths` (trash items are named by them).
fn names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| name(p)).collect()
}

/// "“a.txt”" for one, "3 items" (or "3 paths") for more.
fn count_or_name(what: &str, paths: &[PathBuf]) -> String {
    match paths {
        [p] => format!("\u{201c}{}\u{201d}", name(p)),
        _ => format!("{} {what}s", paths.len()),
    }
}

/// The conflict prompt's card size, and its buttons' width.
const ASK_W: f32 = 460.0;
const ASK_H: f32 = 128.0;
const ASK_BUTTON: f32 = 96.0;
const ASK_CHOICES: [(&str, Option<Conflict>); 4] = [
    ("Cancel", None),
    ("Skip", Some(Conflict::Skip)),
    ("Replace", Some(Conflict::Replace)),
    ("Keep Both", Some(Conflict::KeepBoth)),
];

impl View {
    /// The conflict prompt over the pane: Keep Both (Return), Replace, Skip, Cancel
    /// (Esc). It grows in like the menu.
    pub(super) fn ask(&mut self, ui: &mut Ui, b: &Browser, pane: Rect) {
        let Some((t, n)) = &self.prompt else {
            return;
        };
        let mut pick: Option<Option<Conflict>> = None;
        for k in std::mem::take(&mut ui.s.input.keys) {
            match k.sym {
                0xff0d => pick = Some(Some(Conflict::KeepBoth)),
                0xff1b => pick = Some(None),
                _ => {}
            }
        }
        let grow = ui.spring_from("ask/grow", 0.0, 1.0, SNAP);
        let card = Rect::new(
            pane.x + (pane.w - ASK_W) / 2.0,
            pane.y + (pane.h - ASK_H) / 3.0,
            ASK_W,
            ASK_H,
        )
        .round();
        let shown = Rect {
            h: (card.h * grow).round(),
            ..card
        };
        let raise = ui.pal.raise;
        ui.p.rect(shown, R_PANEL, raise);
        ui.p.clip(Some(shown));
        ask_text(ui, card, (t.dir == b.cwd, &t.dir, *n));
        let y = card.y + card.h - S16 - CONTROL_H;
        for (i, (label, c)) in ASK_CHOICES.into_iter().enumerate() {
            let x = card.x + card.w - S16 - (4 - i) as f32 * (ASK_BUTTON + S8) + S8;
            if ui.button(
                &format!("ask/{i}"),
                Rect::new(x, y, ASK_BUTTON, CONTROL_H),
                label,
                i == 3,
            ) {
                pick = Some(c);
            }
        }
        ui.p.clip(None);
        if let Some(c) = pick {
            ui.forget("ask/grow");
            if let (Some((t, _)), Some(c)) = (self.prompt.take(), c) {
                self.finish_transfer(t, c);
            }
        }
    }
}

/// "2 items already exist here" and what the choices do.
fn ask_text(ui: &mut Ui, card: Rect, (here, dir, n): (bool, &PathBuf, usize)) {
    let place = if here {
        "here".to_string()
    } else {
        format!("in \u{201c}{}\u{201d}", name(dir))
    };
    let what = if n == 1 {
        "1 item already exists".to_string()
    } else {
        format!("{n} items already exist")
    };
    let line = Rect::new(card.x + S16, card.y + S16, card.w - 2.0 * S16, 20.0);
    let strong = ui.strong();
    ui.text(line, &format!("{what} {place}"), strong, Align::Left);
    let hint = "Keep Both renames the new ones; Replace puts the old ones in the Trash.";
    let small = ui.small();
    ui.text(
        Rect {
            y: line.y + 24.0,
            ..line
        },
        hint,
        small,
        Align::Left,
    );
}
