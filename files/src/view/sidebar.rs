//! The places column, full height on its own surface: Home, the XDG folders that
//! exist, the file system, and the free space at the bottom. Below a window width
//! it folds to icons; the change glides.

use std::path::PathBuf;
use std::sync::OnceLock;

use frame::ui::theme::*;
use frame::ui::{Align, Icon, Rect, Ui};

use crate::browse::Browser;
use crate::dir;

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
/// Column width, open and folded to icons.
const OPEN_W: f32 = 208.0;
const FOLDED_W: f32 = CONTROL_H + 2.0 * S8;
/// Below this window width the column shows icons only.
const FOLD_BELOW: f32 = 680.0;
/// A heading line above a group of places.
const HEADING_H: f32 = 28.0;
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
    path: PathBuf,
}

pub struct Places {
    /// Home and its folders, then the file system (one index past `home_end`).
    list: Vec<Place>,
}

impl Places {
    /// Looked up once at start.
    pub fn new() -> Self {
        let home = home();
        let mut list = vec![Place {
            name: "Home".into(),
            icon: Icon::Home,
            path: home.clone(),
        }];
        for (name, icon) in XDG {
            let path = home.join(name);
            if path.is_dir() {
                list.push(Place {
                    name: name.into(),
                    icon,
                    path,
                });
            }
        }
        list.push(Place {
            name: "File System".into(),
            icon: Icon::Drive,
            path: PathBuf::from("/"),
        });
        Places { list }
    }

    /// Entry rects for a column `w` wide, and the headings (y, text): "Places" over
    /// home and its folders, "Devices" over the file system (the last entry).
    fn layout(&self, w: f32) -> (Vec<Rect>, [(f32, &'static str); 2]) {
        let mut heads = [(S16, "Places"), (0.0, "Devices")];
        let mut y = S16 + HEADING_H;
        let last = self.list.len() - 1;
        let rects = (0..self.list.len())
            .map(|i| {
                if i == last {
                    heads[1].0 = y + S12;
                    y += S12 + HEADING_H;
                }
                let r = Rect::new(S8, y, w - 2.0 * S8, CONTROL_H);
                y += CONTROL_H + 2.0;
                r
            })
            .collect();
        (rects, heads)
    }

    /// Draw the column; returns its width this frame (it glides when folding).
    pub fn draw(&mut self, ui: &mut Ui, b: &mut Browser) -> f32 {
        let (win_w, h) = ui.p.size();
        let open = ui.spring("places/open", f32::from(win_w >= FOLD_BELOW), GLIDE);
        let w = (FOLDED_W + (OPEN_W - FOLDED_W) * open).round();
        let side = ui.pal.sidebar;
        let col = Rect::new(0.0, 0.0, w, h);
        ui.p.rect(col, 0.0, side);
        ui.p.clip(Some(col));
        // Names go once the column is too narrow for them: cut at the edge, never squashed.
        let named = open > 0.5;
        let (rects, heads) = self.layout(w);
        if named {
            for (y, title) in heads {
                let ink = ui.heading();
                ui.text(
                    Rect::new(S16, y, w - 2.0 * S16, HEADING_H),
                    title,
                    ink,
                    Align::Left,
                );
            }
        }
        // The current place's surface glides between entries (and fades off-list).
        let current = self.list.iter().position(|p| p.path == b.cwd);
        let raise = ui.pal.raise;
        ui.slide("places/current", current.map(|i| rects[i]), raise);
        let mut go = None;
        for (i, (p, r)) in self.list.iter().zip(rects).enumerate() {
            let label = if named { p.name.as_str() } else { "" };
            if ui.nav(
                &format!("place/{i}"),
                r,
                (p.icon, label),
                current == Some(i),
            ) {
                go = Some(p.path.clone());
            }
        }
        if named && let Some(d) = b.disk {
            disk(
                ui,
                Rect::new(S16, h - S16 - DISK_H, w - 2.0 * S16, DISK_H),
                d,
            );
        }
        ui.p.clip(None);
        if let Some(p) = go {
            b.go(p, None);
        }
        w
    }
}

/// "349 GB free … 378 GB" over a meter of how full the disk is (absolute, ui skill §5).
fn disk(ui: &mut Ui, r: Rect, (free, total): (u64, u64)) {
    let small = ui.small();
    let line = Rect { h: 20.0, ..r };
    ui.text(
        line,
        &format!("{} free", dir::human(free)),
        small,
        Align::Left,
    );
    ui.text(line, &dir::human(total), small, Align::Right);
    let used = 1.0 - free as f32 / total.max(1) as f32;
    ui.meter(Rect::new(r.x, r.y + 28.0, r.w, S4), used);
}
