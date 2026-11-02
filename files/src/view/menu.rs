//! What the right-click menu shows: on selected entries (Open, Open With ›, Cut,
//! Copy …, Compress ›), on the folder (New ›, Paste, Open Terminal Here …) and in the
//! Trash (Restore, Delete Forever, Empty Trash). Picking runs an [`Action`]
//! (actions.rs).

use std::path::PathBuf;

use frame::ui::MenuItem;

use super::actions::Action;
use crate::apps::App;
use crate::bookmarks;
use crate::browse::Browser;
use crate::dir::{self, Kind};
use crate::ops::archive;
use crate::ops::trash;

/// A menu entry: what it does, its label (owned: template names are dynamic), key
/// hint, whether it can be picked now, and its submenu. An empty label is a separator.
pub struct Item {
    pub action: Action,
    label: String,
    hint: &'static str,
    on: bool,
    pub sub: Vec<Item>,
}

fn item(action: Action, label: &str, hint: &'static str, on: bool) -> Item {
    Item {
        action,
        label: label.to_string(),
        hint,
        on,
        sub: Vec::new(),
    }
}

fn sep() -> Item {
    item(Action::Open, "", "", false)
}

/// New files the New menu offers before any templates: label, stem, extension,
/// first content, executable.
pub const NEW_FILES: [(&str, &str, &str, &str, bool); 5] = [
    ("Text File", "New File", "txt", "", false),
    ("Markdown File", "New File", "md", "", false),
    ("Rust File", "new_file", "rs", "", false),
    ("Shell Script", "new_script", "sh", "#!/bin/sh\n", true),
    ("Empty File", "New File", "", "", false),
];

/// "New ›": Folder, the `NEW_FILES` types, then one entry per template.
fn new_menu(templates: &[PathBuf]) -> Item {
    let mut sub = vec![
        item(Action::NewFolder, "Folder", "Ctrl+Shift+N", true),
        sep(),
    ];
    for (i, nf) in NEW_FILES.iter().enumerate() {
        let hint = if i == 0 { "Ctrl+Alt+N" } else { "" };
        sub.push(item(Action::NewFile(i), nf.0, hint, true));
    }
    if !templates.is_empty() {
        sub.push(sep());
    }
    for (i, t) in templates.iter().enumerate() {
        let name = t
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        sub.push(item(Action::NewFile(NEW_FILES.len() + i), &name, "", true));
    }
    Item {
        sub,
        ..item(Action::Open, "New", "", true)
    }
}

/// Apps that open the entry under the menu, and its MIME type.
pub struct OpenWith {
    pub mime: String,
    pub apps: Vec<App>,
    /// The first app is the xdg-mime default.
    pub has_default: bool,
}

/// "Open With ›": the apps (the default marked), then Other Command….
fn open_with_menu(w: &OpenWith) -> Item {
    let mut sub: Vec<Item> = w
        .apps
        .iter()
        .enumerate()
        .map(|(i, a)| {
            item(
                Action::OpenWith(i),
                &a.name,
                if i == 0 && w.has_default {
                    "default"
                } else {
                    ""
                },
                true,
            )
        })
        .collect();
    if !sub.is_empty() {
        sub.push(sep());
    }
    sub.push(item(Action::OpenCommand, "Other Command…", "", true));
    Item {
        sub,
        ..item(Action::Open, "Open With", "", true)
    }
}

/// The menu on selected entries (outside the Trash).
fn entry_items(b: &Browser, can_paste: bool, with: Option<&OpenWith>) -> Vec<Item> {
    let cursor = b.entries.get(b.sel);
    let archive = cursor.is_some_and(|e| dir::kind(e).0 == Kind::Archive);
    let folder = cursor.is_some_and(|e| e.dir);
    let mut v = vec![item(Action::Open, "Open", "Return", true)];
    if folder {
        v.push(item(Action::NewTab, "Open in New Tab", "", true));
    }
    if let Some(w) = with.filter(|_| !folder) {
        v.push(open_with_menu(w));
        if !w.apps.is_empty() {
            v.push(default_menu(w));
        }
    }
    v.push(sep());
    v.push(item(Action::Cut, "Cut", "Ctrl+X", true));
    v.push(item(Action::Copy, "Copy", "Ctrl+C", true));
    v.push(item(Action::Paste, "Paste", "Ctrl+V", can_paste));
    v.push(sep());
    v.push(item(Action::Rename, "Rename…", "F2", true));
    v.push(item(Action::Trash, "Move to Trash", "Delete", true));
    v.push(sep());
    if archive {
        v.push(item(Action::Extract, "Extract Here", "", true));
    }
    let formats = archive::FORMATS.iter().enumerate();
    let sub = formats
        .map(|(i, f)| item(Action::Compress(i), &format!(".{f}"), "", true))
        .collect();
    v.push(Item {
        sub,
        ..item(Action::Open, "Compress", "", true)
    });
    if folder {
        v.push(item(Action::Terminal, "Open in Terminal", "F4", true));
    }
    v.push(item(Action::MakeLink, "Make Link", "", true));
    v
}

/// The menu for the selected entry (`on_item`) or for the folder itself; `templates`
/// are the files in ~/Templates.
pub fn items(
    b: &Browser,
    (can_paste, on_item): (bool, bool),
    (templates, with): (&[PathBuf], Option<&OpenWith>),
) -> Vec<Item> {
    let in_trash = b.cwd == trash::files_dir();
    let sel = b.entries.get(b.sel).filter(|_| on_item);
    let mut v = Vec::new();
    match (sel.is_some(), in_trash) {
        (true, true) => {
            v.push(item(Action::Open, "Open", "Return", true));
            v.push(sep());
            v.push(item(Action::Restore, "Restore", "", true));
            v.push(item(Action::Erase, "Delete Forever", "Delete", true));
        }
        (true, false) => v.extend(entry_items(b, can_paste, with)),
        (false, true) => {
            v.push(item(
                Action::EmptyTrash,
                "Empty Trash",
                "",
                !b.entries.is_empty(),
            ));
        }
        (false, false) => {
            v.push(new_menu(templates));
            v.push(item(Action::Terminal, "Open Terminal Here", "F4", true));
            v.push(item(Action::Paste, "Paste", "Ctrl+V", can_paste));
        }
    }
    v.push(sep());
    if sel.is_none() {
        let hid = if b.hidden {
            "Hide Hidden Files"
        } else {
            "Show Hidden Files"
        };
        v.push(item(Action::Hidden, hid, "Ctrl+H", true));
    }
    v.push(item(Action::Info, "Properties", "Ctrl+I", true));
    v.push(item(Action::CopyPath, "Copy Path", "Ctrl+Shift+C", true));
    let target = sel.map_or(b.cwd.clone(), |e| b.cwd.join(&e.os));
    if sel.is_none_or(|e| e.dir) {
        let mark = if bookmarks::has(&target) {
            "Remove from Bookmarks"
        } else {
            "Add to Bookmarks"
        };
        v.push(item(Action::Bookmark, mark, "Ctrl+D", true));
    }
    v
}

/// A frame menu item for `e` (its submenu items passed in, already built).
pub fn shown<'a>(e: &'a Item, sub: &'a [MenuItem<'a>]) -> MenuItem<'a> {
    match (e.label.is_empty(), e.sub.is_empty()) {
        (true, _) => MenuItem::SEPARATOR,
        (false, true) => MenuItem::new(&e.label, e.hint, e.on),
        (false, false) => MenuItem::submenu(&e.label, sub),
    }
}

/// "Set Default App ›": the apps; one picked opens this file type from now on.
fn default_menu(w: &OpenWith) -> Item {
    let sub = w
        .apps
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mark = if i == 0 && w.has_default {
                "current"
            } else {
                ""
            };
            item(Action::SetDefault(i), &a.name, mark, true)
        })
        .collect();
    Item {
        sub,
        ..item(Action::Open, "Set Default App", "", true)
    }
}
