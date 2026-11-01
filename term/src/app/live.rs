//! Live settings: `~/.config/term/config` and the rice's glass switch
//! (`$XDG_RUNTIME_DIR/rice/glass`, "1" = glass, written by the bar's glass toggle) are
//! watched; a change applies at once to every tab, no restart. Without the switch
//! (or "0") term is solid, like the bar's default.

use std::path::PathBuf;

use super::App;
use crate::color::Palette;
use crate::config::{self, Config};
use crate::font::Font;
use crate::sys::watch::Watch;

/// The folder holding the rice-wide glass switch.
pub(super) fn rice_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("rice"))
}

fn glass_on() -> bool {
    rice_dir()
        .and_then(|d| std::fs::read_to_string(d.join("glass")).ok())
        .is_some_and(|s| s.trim() == "1")
}

/// The config file's settings, made solid while the rice is not in glass mode.
pub fn load() -> Config {
    let (mut cfg, warnings) = Config::load();
    for w in &warnings {
        eprintln!("term: {w}");
    }
    if !glass_on() {
        cfg.theme.opacity = 1.0;
    }
    cfg
}

/// Watch the config folder and the rice folder. None if inotify is unavailable:
/// settings then simply stay as loaded.
pub(super) fn watch() -> Option<Watch> {
    let cfg_dir = config::path()?.parent()?.to_path_buf();
    let dirs: Vec<PathBuf> = [Some(cfg_dir), rice_dir()].into_iter().flatten().collect();
    let refs: Vec<&std::path::Path> = dirs.iter().map(PathBuf::as_path).collect();
    Watch::new(&refs)
        .map_err(|e| eprintln!("term: no live settings: {e}"))
        .ok()
}

impl App {
    /// Apply the current config and glass state to every tab and the window.
    pub(super) fn reload(&mut self) {
        let cfg = load();
        let palette = Palette::new(&cfg.theme);
        for s in &mut self.tabs {
            s.term.set_palette(palette.clone());
        }
        self.renderer.set_theme(cfg.theme);
        self.renderer.pad = cfg.padding;
        let font_changed =
            (cfg.font.as_str(), cfg.font_size) != (self.cfg.font.as_str(), self.cfg.font_size);
        if font_changed {
            match Font::new(&cfg.font, cfg.font_size) {
                Ok(f) => self.renderer.set_font(f),
                Err(e) => eprintln!("term: font: {e}"),
            }
        }
        self.win.set_opaque(cfg.theme.opacity >= 1.0);
        self.cfg = cfg;
        self.layout();
        self.win.invalidate_buffers();
        self.regrid();
    }
}
