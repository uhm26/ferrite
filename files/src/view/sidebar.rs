//! The places column (frame's `Sidebar`): Home, the XDG folders that exist, the
//! Trash, bookmarks, the file system, and the free space at the bottom.

use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

use frame::ui::paint::mix; // mix(0, c, a): c at opacity a (premultiplied, glass-safe)
use frame::ui::theme::S4;
use frame::ui::{Align, Icon, Rect, SideItem, Sidebar, Ui};

use crate::browse::Browser;
use crate::drives::{Drive, Drives, Report};
use crate::ops::trash;
use crate::{bookmarks, dir};

/// Folders under home Nemo lists, if they exist.
/// shortcut: English names; read ~/.config/user-dirs.dirs for renamed/localized ones.
const XDG: [(&str, Icon); 6] = [
    ("Desktop", Icon::Desktop),
    ("Documents", Icon::Documents),
    ("Downloads", Icon::Download),
    ("Music", Icon::Music),
    ("Pictures", Icon::Image),
    ("Videos", Icon::Video),
];
/// The Recent place stands for no folder: this marks it.
const RECENT: &str = "recent:///";
/// Free-space note and meter at the bottom.
const DISK_H: f32 = 40.0;

/// `$HOME`, resolved once (symlinks too, so it matches the canonical folders we show).
pub fn home() -> PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let h = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        std::fs::canonicalize(&h).unwrap_or(h)
    })
    .clone()
}

struct Place {
    name: String,
    icon: Icon,
    /// Its folder (empty for a drive not mounted).
    path: PathBuf,
    /// Its heading: places, bookmarks or devices.
    section: &'static str,
    /// The drive it is (Devices).
    drive: Option<Drive>,
}

pub struct Places {
    /// Places, then bookmarks, then devices.
    list: Vec<Place>,
    /// The column: width, folding, edge drag (the view sets its `content_min`).
    pub side: Sidebar,
    /// Plugged-in drives (udisks2).
    pub drives: Drives,
}

/// The eject button on a mounted drive's row.
const EJECT: f32 = 24.0;

impl Places {
    pub fn new() -> io::Result<Self> {
        let mut p = Places {
            list: Vec::new(),
            side: Sidebar::default(),
            drives: Drives::new()?,
        };
        p.refresh();
        Ok(p)
    }

    /// Look the places up again (a bookmark changed, the trash appeared, a drive).
    pub fn refresh(&mut self) {
        let home = home();
        let place = |name: &str, icon, path, section| Place {
            name: name.into(),
            icon,
            path,
            section,
            drive: None,
        };
        let mut list = vec![
            place("Recent", Icon::Clock, PathBuf::from(RECENT), "Places"),
            place("Home", Icon::Home, home.clone(), "Places"),
        ];
        for (name, icon) in XDG {
            let path = home.join(name);
            if path.is_dir() {
                list.push(place(name, icon, path, "Places"));
            }
        }
        let trash = trash::files_dir();
        if trash.is_dir() {
            list.push(place("Trash", Icon::Trash, trash, "Places"));
        }
        for (name, path) in bookmarks::load() {
            list.push(place(&name, Icon::Folder, path, "Bookmarks"));
        }
        list.push(place("File System", Icon::Drive, "/".into(), "Devices"));
        for d in &self.drives.list {
            let path = d.mount.clone().unwrap_or_default();
            list.push(Place {
                drive: Some(d.clone()),
                ..place(&d.name, Icon::Drive, path, "Devices")
            });
        }
        self.list = list;
    }

    /// The place under `pos` (a drop target): its row and folder.
    pub fn at(&self, pos: (f32, f32)) -> Option<(Rect, PathBuf)> {
        let (i, r) = self.side.at(&items(&self.list), pos)?;
        let path = &self.list[i].path;
        let folder = path.as_os_str() != RECENT && !path.as_os_str().is_empty();
        folder.then(|| (r, path.clone()))
    }

    /// Draw the column; returns its width this frame (it glides when folding).
    pub fn draw(&mut self, ui: &mut Ui, b: &mut Browser) -> f32 {
        let items = items(&self.list);
        let recent = b.search.as_ref().is_some_and(|s| s.recent);
        let current = self.list.iter().position(|p| {
            // Search results belong to no place; the recent files to Recent.
            match p.path.as_os_str() == RECENT {
                true => recent,
                false => b.search.is_none() && p.path == b.cwd,
            }
        });
        let disk_now = b.disk;
        let mut footer = |ui: &mut Ui, r: Rect, a: f32| {
            if let Some(d) = disk_now {
                disk(ui, r, d, a);
            }
        };
        let footer = Some((DISK_H, &mut footer as &mut dyn FnMut(&mut Ui, Rect, f32)));
        let (w, go) = self.side.draw(ui, &items, current, footer);
        // After the rows: an eject press must win over its row's click.
        let eject = self.eject_buttons(ui, &items);
        let ejected = eject.is_some();
        if let Some(d) = eject {
            self.drives.eject(&d);
        }
        match go.filter(|_| !ejected).map(|i| &self.list[i]) {
            Some(p) if p.path.as_os_str() == RECENT => b.show_recent(),
            Some(Place {
                drive: Some(d),
                path,
                ..
            }) if path.as_os_str().is_empty() => self.drives.mount(d),
            Some(p) => {
                b.end_search();
                b.go(p.path.clone(), None);
            }
            None => {}
        }
        w
    }

    /// An eject button on each mounted drive's row while the column is open; the
    /// drive whose button was pressed.
    fn eject_buttons(&self, ui: &mut Ui, items: &[SideItem]) -> Option<Drive> {
        let mut pressed = None;
        for (i, p) in self.list.iter().enumerate() {
            let (Some(d), Some(r)) = (&p.drive, self.side.rect(items, i)) else {
                continue;
            };
            if d.mount.is_none() || r.w < 4.0 * EJECT {
                continue;
            }
            let at = Rect::new(
                r.x + r.w - EJECT - S4,
                r.y + (r.h - EJECT) / 2.0,
                EJECT,
                EJECT,
            );
            if ui.icon_button(&format!("eject/{}", d.name), at, Icon::Eject, true) {
                pressed = Some(d.clone());
            }
        }
        pressed
    }

    /// Take in what the drives worker reported; what to tell the owner, if anything.
    pub fn drive_reports(&mut self, b: &mut Browser) -> Option<String> {
        let mut note = None;
        for r in self.drives.reports() {
            match r {
                Report::List(_) => self.refresh(),
                Report::Mounted(at) => {
                    b.end_search();
                    b.go(at, None);
                }
                Report::Ejected(name) => {
                    note = Some(format!("\u{201c}{name}\u{201d} can be unplugged now"))
                }
                Report::Unmounted(name) => note = Some(format!("Unmounted \u{201c}{name}\u{201d}")),
                Report::Failed(e) => note = Some(failed(&e)),
            }
        }
        note
    }
}

/// "349 GB free … 378 GB" over a meter of how full the disk is (absolute, ui skill §5),
/// at opacity `a` (it fades as the column folds).
fn disk(ui: &mut Ui, r: Rect, (free, total): (u64, u64), a: f32) {
    let small = ui.small();
    let small = small.color(mix(0, small.color, a));
    let line = Rect { h: 20.0, ..r };
    ui.text(
        line,
        &format!("{} free", dir::human(free)),
        small,
        Align::Left,
    );
    ui.text(line, &dir::human(total), small, Align::Right);
    let used = 1.0 - free as f32 / total.max(1) as f32;
    // ui.meter's look, faded.
    let bar = Rect::new(r.x, r.y + 28.0, r.w, S4).round();
    let fill = Rect {
        w: (bar.w * used.clamp(0.0, 1.0)).round().max(bar.h),
        ..bar
    };
    let (raise, sub) = (ui.pal.raise, ui.pal.sub);
    ui.p.rect(bar, bar.h / 2.0, mix(0, raise, a));
    ui.p.rect(fill, bar.h / 2.0, mix(0, sub, a));
}

/// The sidebar entries for `list`.
fn items(list: &[Place]) -> Vec<SideItem<'_>> {
    list.iter()
        .map(|p| SideItem {
            icon: p.icon,
            label: &p.name,
            section: p.section,
        })
        .collect()
}

/// A failed mount or eject, in words: udisks wants a password for internal disks and
/// we have no polkit agent to ask for it.
fn failed(e: &str) -> String {
    if e.contains("NotAuthorized") {
        "Error: not allowed to mount this disk without a password (needs a polkit rule)".into()
    } else {
        format!("Error: {e}")
    }
}
