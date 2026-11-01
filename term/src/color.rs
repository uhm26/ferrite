//! The color palette: 16 theme colors, the xterm 256-color cube and grays, and the
//! default fg/bg/cursor. Turns a cell's symbolic color into 0x00RRGGBB.

use crate::term::cell::{Color, Kind};

/// Every color the terminal shows: the text colors plus the tab bar (UI).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub ansi: [u32; 16],
    pub fg: u32,
    pub bg: u32,
    pub cursor: u32,
    /// Background of selected text (text keeps its color).
    pub selection: u32,
    /// Tab bar background, hairlines, inactive and active tab text.
    pub tab_bg: u32,
    pub tab_line: u32,
    pub tab_inactive: u32,
    pub tab_active: u32,
    /// Opacity of the default background and the tab bar (1 = solid). Below 1 the
    /// compositor's blur shows through (glass, like the bar).
    pub opacity: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub ansi: [u32; 256],
    pub fg: u32,
    pub bg: u32,
    pub cursor: u32,
    pub selection: u32,
    /// Alpha of the default background (from `Theme::opacity`).
    pub alpha: u8,
}

/// An opaque pixel (ARGB8888).
pub const fn px(rgb: u32) -> u32 {
    0xff00_0000 | rgb
}

/// A see-through pixel: `rgb` at `alpha`, premultiplied as wl_shm ARGB8888 expects.
pub fn glass(rgb: u32, alpha: u8) -> u32 {
    let a = u32::from(alpha);
    let ch = |s: u32| (((rgb >> s & 0xff) * a + 127) / 255) << s;
    a << 24 | ch(16) | ch(8) | ch(0)
}

impl Palette {
    pub fn new(t: &Theme) -> Self {
        let mut ansi = [0; 256];
        ansi[..16].copy_from_slice(&t.ansi);
        for (i, slot) in ansi.iter_mut().enumerate().skip(16) {
            *slot = xterm256(i as u8);
        }
        Palette {
            ansi,
            fg: t.fg,
            bg: t.bg,
            cursor: t.cursor,
            selection: t.selection,
            alpha: (t.opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
        }
    }

    /// Foreground pixel (always opaque): bold makes colors 0-7 bright, like most
    /// terminals.
    pub fn fg(&self, c: Color, bold: bool) -> u32 {
        px(match c.kind() {
            Kind::Rgb(v) => v,
            Kind::Index(i) if bold && i < 8 => self.ansi[usize::from(i) + 8],
            Kind::Index(i) => self.ansi[usize::from(i)],
            Kind::Default => self.fg,
        })
    }

    /// Background pixel: only the default background is see-through; colors an app
    /// asked for stay solid (as in kitty).
    pub fn bg(&self, c: Color) -> u32 {
        match c.kind() {
            Kind::Rgb(v) => px(v),
            Kind::Index(i) => px(self.ansi[usize::from(i)]),
            Kind::Default => self.bg_px(),
        }
    }

    pub fn bg_px(&self) -> u32 {
        glass(self.bg, self.alpha)
    }

    pub fn cursor_px(&self) -> u32 {
        px(self.cursor)
    }

    pub fn selection_px(&self) -> u32 {
        px(self.selection)
    }
}

/// Colors 16..=255 of the xterm palette: a 6×6×6 cube, then 24 grays.
fn xterm256(i: u8) -> u32 {
    if i >= 232 {
        let v = 8 + 10 * u32::from(i - 232);
        return v << 16 | v << 8 | v;
    }
    let i = u32::from(i - 16);
    let level = |n: u32| if n == 0 { 0 } else { 55 + 40 * n };
    level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6)
}

/// X color spec: `#rrggbb` or `rgb:r/g/b` (1-4 hex digits each).
pub fn parse(spec: &str) -> Option<u32> {
    if let Some(hex) = spec.strip_prefix('#') {
        return (hex.len() == 6)
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten();
    }
    let parts: Vec<&str> = spec.strip_prefix("rgb:")?.split('/').collect();
    let [r, g, b] = parts[..] else { return None };
    let chan = |s: &str| -> Option<u32> {
        let v = u32::from_str_radix(s, 16).ok()?;
        let max = (1u32 << (4 * s.len().clamp(1, 4))) - 1;
        Some(v * 255 / max)
    };
    Some(chan(r)? << 16 | chan(g)? << 8 | chan(b)?)
}

/// The reply form for OSC 4/10/11 queries: `rgb:rrrr/gggg/bbbb`.
pub fn spec(v: u32) -> String {
    let c = |s: u32| (v >> s & 0xff) * 0x101;
    format!("rgb:{:04x}/{:04x}/{:04x}", c(16), c(8), c(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grays() {
        assert_eq!(xterm256(16), 0x000000);
        assert_eq!(xterm256(196), 0xff0000);
        assert_eq!(xterm256(231), 0xffffff);
        assert_eq!(xterm256(232), 0x080808);
        assert_eq!(xterm256(255), 0xeeeeee);
    }

    #[test]
    fn glass_is_premultiplied() {
        assert_eq!(glass(0xffffff, 255), 0xffff_ffff);
        assert_eq!(glass(0xff8000, 128), 0x8080_4000);
        assert_eq!(glass(0x123456, 0), 0);
        assert_eq!(px(0x0d0d0d), 0xff0d_0d0d);
    }

    #[test]
    fn specs() {
        assert_eq!(parse("#0d0d0d"), Some(0x0d0d0d));
        assert_eq!(parse("rgb:ff/80/00"), Some(0xff8000));
        assert_eq!(parse("rgb:ffff/0000/8080"), Some(0xff0080));
        assert_eq!(parse("nope"), None);
        assert_eq!(spec(0x0d0d0d), "rgb:0d0d/0d0d/0d0d");
    }
}
