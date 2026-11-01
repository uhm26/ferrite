//! The toolbar: back, forward and up, the path as clickable crumbs (home or the
//! root as an icon first), and the list/grid switch on the right. A path wider
//! than the room loses its leading crumbs behind one `…`.

use std::path::{Path, PathBuf};

use frame::ui::theme::*;
use frame::ui::{Icon, Rect, Ui};

use super::location::Location;
use super::sidebar::home;
use crate::browse::Browser;

/// Chevron between crumbs.
const SEP: f32 = 12.0;

/// Draw the bar in `r`; true if the view switch changed `mode`.
pub fn draw(ui: &mut Ui, b: &mut Browser, r: Rect, mode: &mut usize, loc: &mut Location) -> bool {
    let nav = [
        (Icon::Back, b.can_back()),
        (Icon::Forward, b.can_forward()),
        (Icon::Up, b.cwd.parent().is_some()),
    ];
    let mut x = r.x;
    for (i, (icon, on)) in nav.into_iter().enumerate() {
        let br = Rect::new(x, r.y, CONTROL_H, CONTROL_H);
        if ui.icon_button(&format!("nav/{i}"), br, icon, on) {
            match i {
                0 => b.travel(true),
                1 => b.travel(false),
                _ => b.up(),
            }
        }
        x += CONTROL_H + S4;
    }
    let seg_w = Ui::segment_w(2);
    let seg_x = r.x + r.w - seg_w;
    let changed = ui.segment("view", (seg_x, r.y), &[Icon::List, Icon::Grid], mode);
    let room = Rect::new(x + S8, r.y, (seg_x - S12 - x - S8).max(0.0), r.h);
    if loc.open {
        loc.draw(ui, b, room);
        return changed;
    }
    // Empty space in the path bar opens the location field (crumbs drawn after win
    // their own clicks).
    let edit = ui.clicked("crumbs/space", room);
    if crumbs(ui, b, room) {
        return changed;
    }
    if edit {
        // Open and draw the field in this same frame: no wait for the next event.
        loc.start(ui, b, None);
        loc.draw(ui, b, room);
    }
    changed
}

/// One ghost button per folder from home (or `/`) down to where we are. True if a
/// crumb was clicked.
fn crumbs(ui: &mut Ui, b: &mut Browser, r: Rect) -> bool {
    let trail = trail(&b.cwd, &home());
    let widths: Vec<f32> = trail
        .iter()
        .map(|(icon, label, _)| {
            if icon.is_some() {
                CONTROL_H
            } else {
                ui.ghost_w(label)
            }
        })
        .collect();
    // Drop leading crumbs (after the first) until the rest fits; `…` stands in.
    let ellipsis_w = ui.ghost_w("…") + SEP;
    let total = |from: usize| -> f32 {
        let skipped = if from > 1 { ellipsis_w } else { 0.0 };
        widths[0] + skipped + widths[from.max(1)..].iter().map(|w| w + SEP).sum::<f32>()
    };
    let from = (1..trail.len())
        .find(|&f| total(f) <= r.w)
        .unwrap_or(trail.len() - 1);
    let mut x = r.x;
    let mut go = None;
    ui.p.clip(Some(r));
    for (i, (icon, label, path)) in trail.iter().enumerate() {
        if i > 0 && i < from {
            if i == from - 1 {
                x = sep(ui, x, r);
                let er = Rect::new(x, r.y, ellipsis_w - SEP, r.h);
                if ui.ghost("crumb/more", er, "…", false) {
                    go = Some(path.clone());
                }
                x += er.w;
            }
            continue;
        }
        if i > 0 {
            x = sep(ui, x, r);
        }
        let cr = Rect::new(x, r.y, widths[i], r.h);
        let current = i + 1 == trail.len();
        let hit = match icon {
            Some(icon) => ui.icon_button(&format!("crumb/{i}"), cr, *icon, true),
            None => ui.ghost(&format!("crumb/{i}"), cr, label, current),
        };
        if hit && !current {
            go = Some(path.clone());
        }
        x += cr.w;
    }
    ui.p.clip(None);
    let went = go.is_some();
    if let Some(p) = go {
        b.go(p, None);
    }
    went
}

/// A small chevron at `x`; returns the x after it.
fn sep(ui: &mut Ui, x: f32, r: Rect) -> f32 {
    let dim = ui.pal.dim;
    ui.icon(Icon::Forward, Rect::new(x, r.y, SEP, r.h), SEP, dim);
    x + SEP
}

/// The crumbs for `cwd`: an icon for home (or `/` outside it) first, then one
/// named crumb per folder below it, each with the path it opens.
fn trail(cwd: &Path, home: &Path) -> Vec<(Option<Icon>, String, PathBuf)> {
    let (mut at, rest, icon) = match cwd.strip_prefix(home) {
        Ok(rest) if home != Path::new("/") => (home.to_path_buf(), rest, Icon::Home),
        _ => (
            PathBuf::from("/"),
            cwd.strip_prefix("/").unwrap_or(cwd),
            Icon::Drive,
        ),
    };
    let mut out = vec![(Some(icon), String::new(), at.clone())];
    for part in rest.iter() {
        at.push(part);
        out.push((None, part.to_string_lossy().into_owned(), at.clone()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crumbs_start_at_home_or_root() {
        let labels = |cwd: &str| -> Vec<String> {
            trail(Path::new(cwd), Path::new("/home/gabe"))
                .into_iter()
                .map(|c| {
                    if c.0 == Some(Icon::Home) {
                        "~".into()
                    } else if c.0.is_some() {
                        "/".into()
                    } else {
                        c.1
                    }
                })
                .collect()
        };
        assert_eq!(
            labels("/home/gabe/projects/files"),
            ["~", "projects", "files"]
        );
        assert_eq!(labels("/home/gabe"), ["~"]);
        assert_eq!(labels("/usr/lib"), ["/", "usr", "lib"]);
        assert_eq!(labels("/home/gabe2"), ["/", "home", "gabe2"]);
        assert_eq!(labels("/"), ["/"]);
    }
}
