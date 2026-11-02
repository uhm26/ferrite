//! The info pane (Ctrl+I, or Properties in the menu), on the right of the list: the
//! selected entry's kind, size (a folder's total is added up in the background),
//! dates, permissions and place, and the first lines of a text file. Several
//! selected: their count and total size.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use frame::ui::paint::mix;
use frame::ui::text::REGULAR;
use frame::ui::theme::*;
use frame::ui::{Align, Ink, Rect, Ui};

use super::View;
use crate::browse::Browser;
use crate::dir::{self, Entry, Kind};
use crate::ops::Op;

/// The pane's width when open (it glides in and out).
pub const INFO_W: f32 = 288.0;
/// Text previews read at most this much.
const PREVIEW_BYTES: usize = 8 * 1024;
/// A label/value line.
const LINE_H: f32 = 22.0;

/// What the pane shows about one file, read once per file version.
pub struct Details {
    path: PathBuf,
    modified: i64,
    mode: u32,
    preview: Vec<String>,
}

impl Details {
    fn read(path: &Path, e: &Entry) -> Self {
        let mode = fs::metadata(path).map_or(0, |m| m.permissions().mode());
        let texty = matches!(dir::kind(e).0, Kind::Text | Kind::Code | Kind::Other);
        let preview = if !e.dir && texty {
            preview(path)
        } else {
            Vec::new()
        };
        Details {
            path: path.to_path_buf(),
            modified: e.modified,
            mode,
            preview,
        }
    }
}

/// The first lines of a text file (none if it isn't valid UTF-8 text).
fn preview(path: &Path) -> Vec<String> {
    use std::io::Read;
    let mut buf = vec![0u8; PREVIEW_BYTES];
    let n = fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    buf.truncate(n);
    // A cut multi-byte char at the end is fine; anything else invalid is not text.
    let text = match std::str::from_utf8(&buf) {
        Ok(t) => t,
        Err(e) if e.error_len().is_none() => {
            std::str::from_utf8(&buf[..e.valid_up_to()]).unwrap_or("")
        }
        Err(_) => return Vec::new(),
    };
    if text.contains('\0') {
        return Vec::new();
    }
    text.lines()
        .take(60)
        .map(|l| l.replace('\t', "    "))
        .collect()
}

impl View {
    /// Draw the pane in `r` for the selection.
    pub(super) fn info(&mut self, ui: &mut Ui, b: &Browser, r: Rect) {
        ui.p.clip(Some(r));
        let sel = b.selection();
        let inner = Rect::new(r.x + S12, r.y + S12, r.w - 2.0 * S12, r.h - 2.0 * S12);
        match sel.as_slice() {
            [] => {}
            [i] => self.one(ui, b, (&b.entries[*i], inner)),
            many => {
                let bytes: u64 = many.iter().map(|&i| b.entries[i].size).sum();
                let ink = ui.strong();
                ui.text(
                    Rect { h: LINE_H, ..inner },
                    &format!("{} items selected", many.len()),
                    ink,
                    Align::Left,
                );
                let at = Rect {
                    y: inner.y + LINE_H + S8,
                    ..inner
                };
                line(ui, at, ("Files total", &dir::human(bytes)));
            }
        }
        ui.p.clip(None);
    }

    fn one(&mut self, ui: &mut Ui, b: &Browser, (e, r): (&Entry, Rect)) {
        let path = b.cwd.join(&e.os);
        if self
            .details
            .as_ref()
            .is_none_or(|d| d.path != path || d.modified != e.modified)
        {
            self.details = Some(Details::read(&path, e));
        }
        let icon_box = Rect { h: 64.0, ..r };
        let color = super::icon_color(ui, e, 0.0);
        ui.icon(super::icon(e), icon_box, 2.0 * ICON, color);
        let strong = ui.strong();
        ui.text(
            Rect::new(r.x, r.y + 64.0, r.w, LINE_H),
            &e.name,
            strong,
            Align::Center,
        );
        let size = self.size_text(e, &path);
        let d = self.details.as_ref();
        let rows = [
            ("Type", dir::kind(e).1),
            ("Size", size),
            ("Modified", dir::date_full(e.modified)),
            (
                "Permissions",
                d.map_or(String::new(), |d| dir::mode_text(d.mode)),
            ),
            // The folder it is in (search results come from subfolders).
            (
                "Location",
                path.parent().unwrap_or(&b.cwd).display().to_string(),
            ),
        ];
        let mut y = r.y + 64.0 + LINE_H + S12;
        for (label, value) in &rows {
            line(ui, Rect { y, h: LINE_H, ..r }, (label, value));
            y += LINE_H;
            if *label == "Permissions" {
                y = self.perms(ui, (r, y), (&path, e.dir));
            }
        }
        let lines = self
            .details
            .as_ref()
            .map_or(&[][..], |d| d.preview.as_slice());
        if !lines.is_empty() {
            preview_block(ui, Rect::new(r.x, y + S12, r.w, r.y + r.h - y - S12), lines);
        }
    }

    /// "4.2 KB" for a file; for a folder its total once measured (asking for it once).
    fn size_text(&mut self, e: &Entry, path: &Path) -> String {
        if !e.dir {
            return format!("{} ({} bytes)", dir::human(e.size), e.size);
        }
        if let Some(&(bytes, files)) = self.sizes.get(path) {
            return format!("{}, {} files", dir::human(bytes), files);
        }
        if self.measuring.insert(path.to_path_buf()) {
            let _ = self.jobs.start(Op::Measure(path.to_path_buf()));
        }
        "Measuring…".into()
    }
}

/// Permission chips: who, and the bit shift of their rwx triple.
const WHO: [(&str, u32); 3] = [("Owner", 6), ("Group", 3), ("Others", 0)];
/// A permission chip's width.
const CHIP_W: f32 = 52.0;

impl View {
    /// Read / Write / Run chips for owner, group and others, from `y`; a click flips
    /// that bit on disk. Returns the y below them.
    fn perms(&mut self, ui: &mut Ui, (r, y): (Rect, f32), (path, dir): (&Path, bool)) -> f32 {
        let Some(mode) = self.details.as_ref().map(|d| d.mode) else {
            return y;
        };
        let run = if dir { "Open" } else { "Run" };
        let mut y = y;
        for (who, shift) in WHO {
            let small = ui.small();
            ui.text(Rect::new(r.x, y, 88.0, LINE_H), who, small, Align::Left);
            for (k, (what, bit)) in [("Read", 4), ("Write", 2), (run, 1)]
                .into_iter()
                .enumerate()
            {
                let c = Rect::new(
                    r.x + 92.0 + k as f32 * (CHIP_W + S4),
                    y + 1.0,
                    CHIP_W,
                    LINE_H - 2.0,
                );
                let flag = bit << shift;
                if chip(ui, (&format!("perm/{flag}"), c), what, mode & flag != 0) {
                    self.set_mode(path, mode ^ flag);
                }
            }
            y += LINE_H;
        }
        y
    }

    /// Change `path`'s permission bits (the owner or root only).
    fn set_mode(&mut self, path: &Path, mode: u32) {
        match fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777)) {
            Ok(()) => {
                if let Some(d) = self.details.as_mut() {
                    d.mode = mode;
                }
            }
            Err(e) => self.note = Some(format!("Error: {}: {e}", path.display())),
        }
    }
}

/// A small toggle pill: filled when on, a hairline when off; on and hover glide.
/// True when clicked.
fn chip(ui: &mut Ui, (name, r): (&str, Rect), text: &str, on: bool) -> bool {
    let clicked = ui.clicked(name, r);
    let over = ui.hovered(r);
    let lit = ui.spring(&format!("{name}/on"), f32::from(on), SNAP);
    let hot = ui.spring(&format!("{name}/hover"), f32::from(over), SNAP);
    let pal = ui.pal;
    let r = r.round();
    let rad = r.h / 2.0;
    let fill = mix(mix(0, pal.hover, hot), pal.selected, lit);
    ui.p.rect(r, rad, fill);
    ui.p.stroke(r, rad, 1.0, mix(0, pal.line, 1.0 - lit));
    let ink = Ink::new(SMALL, REGULAR, mix(pal.sub, pal.text, lit.max(hot)));
    ui.text(r, text, ink, Align::Center);
    clicked
}

/// "Label    value" on one line: the label dim, the value cut to fit.
fn line(ui: &mut Ui, r: Rect, (label, value): (&str, &str)) {
    let small = ui.small();
    ui.text(Rect { w: 88.0, ..r }, label, small, Align::Left);
    let body = Ink::new(SMALL, REGULAR, ui.pal.text);
    ui.text(
        Rect {
            x: r.x + 92.0,
            w: r.w - 92.0,
            ..r
        },
        value,
        body,
        Align::Left,
    );
}

/// A text file's first lines on a panel, as many as fit.
fn preview_block(ui: &mut Ui, r: Rect, lines: &[String]) {
    if r.h < 40.0 {
        return;
    }
    let panel = ui.pal.panel;
    ui.p.rect(r, RADIUS, panel);
    let ink = Ink::new(SMALL, REGULAR, ui.pal.sub);
    let fit = ((r.h - 2.0 * S8) / 16.0) as usize;
    for (k, l) in lines.iter().take(fit).enumerate() {
        let lr = Rect::new(r.x + S8, r.y + S8 + k as f32 * 16.0, r.w - 2.0 * S8, 16.0);
        ui.text(lr, l, ink, Align::Left);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_text_but_not_binary() {
        let d = std::env::temp_dir().join(format!("files-info-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("t.md"), "# hi\n\tline two\n").unwrap();
        fs::write(d.join("b.bin"), [0u8, 159, 146, 150]).unwrap();
        let (t, bin) = (preview(&d.join("t.md")), preview(&d.join("b.bin")));
        fs::remove_dir_all(&d).unwrap();
        assert_eq!(t, ["# hi", "    line two"]);
        assert!(bin.is_empty());
    }
}
