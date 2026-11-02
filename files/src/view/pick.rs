//! Picker mode (`files --pick open|dir|save`, for the portal in portal.rs): the same
//! window, plus Cancel and an accept button in the toolbar, and in save mode a name
//! field. Opening a file picks it instead of starting its app; a folder still opens.
//! The app prints what was picked and quits.

use std::path::PathBuf;

use frame::ui::theme::*;
use frame::ui::{Rect, Ui};

use super::View;
use crate::browse::Browser;

const FIELD: &str = "pick/name";
const NAME_W: f32 = 220.0;
const BUTTON_W: f32 = 96.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// One or more files.
    Open,
    /// A folder: the one shown.
    Dir,
    /// A new file name in the folder shown.
    Save,
}

pub struct Pick {
    pub mode: Mode,
    pub multiple: bool,
    /// The accept button's text ("Open", "Upload", …).
    pub accept: String,
    /// The typed name (save).
    pub name: String,
    /// Save asked to replace this file; the same again does it.
    replace: Option<PathBuf>,
    /// The name was focused once (it starts focused, the stem selected).
    focused: bool,
}

impl Pick {
    /// `mode` "open", "dir" or "save"; None for anything else.
    pub fn new(mode: &str, multiple: bool, accept: Option<String>, name: String) -> Option<Pick> {
        let (mode, default) = match mode {
            "open" => (Mode::Open, "Open"),
            "dir" => (Mode::Dir, "Select"),
            "save" => (Mode::Save, "Save"),
            _ => return None,
        };
        Some(Pick {
            mode,
            multiple,
            accept: accept.unwrap_or_else(|| default.into()),
            name,
            replace: None,
            focused: false,
        })
    }
}

impl View {
    pub fn set_pick(&mut self, p: Pick) {
        self.pick = Some(p);
    }

    /// Room the picker's controls take in the toolbar (0 when not picking).
    pub(super) fn pick_w(&self) -> f32 {
        match &self.pick {
            None => 0.0,
            Some(p) if p.mode == Mode::Save => NAME_W + S8 + 2.0 * BUTTON_W + S8 + S12,
            Some(_) => 2.0 * BUTTON_W + S8 + S12,
        }
    }

    /// The name field typed into (keys belong to it).
    pub(super) fn pick_typing(&self, ui: &Ui) -> bool {
        self.pick.is_some() && ui.focused(FIELD)
    }

    /// Name field (save), Cancel and accept, ending at `right` in the toolbar row `bar`.
    pub(super) fn pick_bar(&mut self, ui: &mut Ui, b: &Browser, (bar, right): (Rect, f32)) {
        let Some(p) = self.pick.as_mut() else {
            return;
        };
        let accept_r = Rect::new(right - BUTTON_W, bar.y, BUTTON_W, CONTROL_H);
        let cancel_r = Rect {
            x: accept_r.x - S8 - BUTTON_W,
            ..accept_r
        };
        if p.mode == Mode::Save {
            let r = Rect::new(cancel_r.x - S8 - NAME_W, bar.y, NAME_W, CONTROL_H);
            if !p.focused {
                // Typing replaces the name, not its extension.
                let stem = p.name.rfind('.').filter(|&i| i > 0).unwrap_or(p.name.len());
                ui.take_focus_range(FIELD, 0, stem);
                p.focused = true;
            }
            if ui.field(FIELD, r, &mut p.name, "File name") {
                p.replace = None;
            }
        }
        if ui.button("pick/cancel", cancel_r, "Cancel", false) {
            self.quit = true;
        }
        let submitted = ui.submitted(FIELD);
        if ui.button("pick/accept", accept_r, &p.accept.clone(), true) || submitted {
            self.accept(b);
        }
    }

    /// The accept button: what is picked, if anything can be.
    fn accept(&mut self, b: &Browser) {
        let Some(p) = self.pick.as_mut() else {
            return;
        };
        let paths = match p.mode {
            Mode::Dir => vec![b.cwd.clone()],
            Mode::Open => {
                let mut files: Vec<PathBuf> = b
                    .selected_paths()
                    .into_iter()
                    .filter(|f| !f.is_dir())
                    .collect();
                if !p.multiple {
                    files.truncate(1);
                }
                files
            }
            Mode::Save => {
                let name = p.name.trim();
                if name.is_empty() || name.contains('/') {
                    return;
                }
                let to = b.cwd.join(name);
                if to.exists() && p.replace.as_ref() != Some(&to) {
                    self.note = Some(format!(
                        "\u{201c}{name}\u{201d} exists: Save again to replace it"
                    ));
                    p.replace = Some(to);
                    return;
                }
                vec![to]
            }
        };
        if !paths.is_empty() {
            self.picked = Some(paths);
            self.quit = true;
        }
    }

    /// Opening the selected entry while picking: a file is picked (Open mode), or in
    /// save mode its name is taken. True if handled (else open it as usual).
    pub(super) fn pick_open(&mut self, b: &Browser) -> bool {
        let Some(p) = self.pick.as_mut() else {
            return false;
        };
        let Some(e) = b.entries.get(b.sel).filter(|e| !e.dir) else {
            return false;
        };
        match p.mode {
            Mode::Open => self.accept(b),
            Mode::Save => {
                p.name.clone_from(&e.name);
                self.accept(b);
            }
            Mode::Dir => {}
        }
        true
    }

    /// Open the selected entry: a folder, a file's app, or while picking pick it.
    pub(super) fn open_entry(&mut self, b: &mut Browser) {
        if !self.pick_open(b) {
            b.open();
        }
    }

    /// What was picked, once the window quits (None: cancelled).
    pub fn picked(&self) -> Option<&[PathBuf]> {
        self.picked.as_deref()
    }
}

impl Pick {
    /// Flags of `files --pick` that take a value.
    pub const TAKES_VALUE: &[&str] = &["--pick", "--title", "--accept", "--name"];

    /// `--pick open|dir|save [--multiple] [--title T] [--accept LABEL] [--name FILE]`:
    /// the picker and its window title. None without `--pick`.
    pub fn from_args(args: &[&str]) -> Option<(Pick, String)> {
        let val = |f: &str| {
            let i = args.iter().position(|a| *a == f)?;
            args.get(i + 1).map(|v| v.to_string())
        };
        let multiple = args.contains(&"--multiple");
        let p = Pick::new(&val("--pick")?, multiple, val("--accept"), val("--name").unwrap_or_default())?;
        let title = val("--title").unwrap_or_else(|| p.accept.clone());
        Some((p, title))
    }
}

impl View {
    /// Picking (the window keeps the title the app asked for).
    pub fn pick_title(&self) -> Option<&str> {
        self.pick.as_ref().map(|p| p.accept.as_str())
    }
}
