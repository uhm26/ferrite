//! Settings from `~/.config/term/config` (`key = value`; lines starting with `#` are
//! comments). Every key is optional; bad lines are reported and skipped, never
//! fatal. The default theme follows the hypr rice
//! (../hypr/docs/plans/look/2026-11-01-rice-vision.md): near-black, gray text,
//! one warm sand accent, 1 px hairlines, low-chroma colors.

use std::fmt;
use std::path::PathBuf;

use crate::color::{self, Palette, Theme};

/// "Ash": the rice's grays plus low-chroma hues of similar lightness, so colored
/// output reads calm on #0d0d0d. Yellow is the rice accent (sand).
pub const ASH: Theme = Theme {
    ansi: [
        0x1e1e1e, // black
        0xc27272, // red: muted brick
        0x93a48b, // green: sage
        0xd9c8af, // yellow: sand (the rice accent)
        0x8c9cb0, // blue: slate
        0xab95aa, // magenta: mauve
        0x8ba7a5, // cyan: muted teal
        0xbcbcbc, // white
        0x6a6a6a, // bright black: rice "dim" (fish autosuggestions)
        0xd48d8d, 0xaabba2, 0xe8dcc8, 0xa6b3c4, 0xc2aec1, 0xa7c0be, 0xf0f0f0,
    ],
    fg: 0xd4d4d4,
    bg: 0x0d0d0d,
    cursor: 0xd9c8af,
    selection: 0x34302a,
    tab_bg: 0x151515,
    tab_line: 0x2a2a2a,
    tab_inactive: 0x6a6a6a,
    tab_active: 0xd4d4d4,
    // Glass like the bar (it uses a 35 % tint); a terminal needs a bit more dark
    // behind text to stay readable on any wallpaper.
    opacity: 0.65,
};

#[derive(Clone, Debug)]
pub struct Config {
    pub font: String,
    pub font_size: f32,
    pub scrollback: usize,
    pub padding: usize,
    pub shell: Option<String>,
    pub theme: Theme,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            font: "Adwaita Mono".into(),
            font_size: 11.0,
            scrollback: 10_000,
            padding: 6,
            shell: None,
            theme: ASH,
        }
    }
}

/// One rejected line.
#[derive(Debug)]
pub struct Warning {
    pub line: usize,
    pub text: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "config line {}: {}", self.line, self.text)
    }
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("term/config"))
}

impl Config {
    /// Defaults, overridden by the config file if there is one.
    pub fn load() -> (Self, Vec<Warning>) {
        let text = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> (Self, Vec<Warning>) {
        let mut cfg = Config::default();
        let mut warnings = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Err(text) = cfg.set(line) {
                warnings.push(Warning { line: i + 1, text });
            }
        }
        (cfg, warnings)
    }

    fn set(&mut self, line: &str) -> Result<(), String> {
        let (key, value) = line
            .split_once('=')
            .map(|(k, v)| (k.trim(), v.trim()))
            .ok_or_else(|| format!("expected `key = value`, got {line:?}"))?;
        let num = || {
            value
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0) // no NaN, inf or negatives
                .ok_or_else(|| format!("bad number {value:?}"))
        };
        match key {
            "font" => self.font = value.to_string(),
            "font_size" => self.font_size = (num()? as f32).clamp(4.0, 72.0),
            "scrollback" => self.scrollback = (num()? as usize).min(1_000_000),
            "padding" => self.padding = (num()? as usize).min(200),
            "opacity" => self.theme.opacity = (num()? as f32).clamp(0.0, 1.0),
            "shell" => self.shell = Some(value.to_string()),
            _ => {
                let slot = self.color_slot(key)?;
                *slot = color::parse(value).ok_or_else(|| format!("bad color {value:?}"))?;
            }
        }
        Ok(())
    }

    fn color_slot(&mut self, key: &str) -> Result<&mut u32, String> {
        let t = &mut self.theme;
        Ok(match key {
            "foreground" => &mut t.fg,
            "background" => &mut t.bg,
            "cursor" => &mut t.cursor,
            "selection" => &mut t.selection,
            "tab_background" => &mut t.tab_bg,
            "tab_line" => &mut t.tab_line,
            "tab_inactive" => &mut t.tab_inactive,
            "tab_active" => &mut t.tab_active,
            _ => key
                .strip_prefix("color")
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| t.ansi.get_mut(n))
                .ok_or_else(|| format!("unknown key {key:?}"))?,
        })
    }

    pub fn palette(&self) -> Palette {
        Palette::new(&self.theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_warns() {
        let text = "font = JetBrains Mono\n# comment\nfont_size=13\ncolor1 = #ff0000\nnope\nx = 1\nselection = #112233\n";
        let (c, w) = Config::parse(text);
        assert_eq!(c.font, "JetBrains Mono");
        assert_eq!(c.font_size, 13.0);
        assert_eq!(c.theme.ansi[1], 0xff0000);
        assert_eq!(c.theme.selection, 0x112233);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].line, 5);
    }
}
