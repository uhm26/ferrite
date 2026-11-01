//! Design tokens, from the ui skill (~/.claude/skills/ui). Colors are premultiplied
//! ARGB. All sizes are logical px and never scale with the window.
//!
//! The look (owner 2026-11-01, "boltsnap flat", not a 1:1 clone): flat surfaces,
//! depth only from gray steps (no shadows, no outlines), hierarchy from weight and
//! text color, one inverted emphasis (light on dark flips to dark on light).

// Surfaces, darkest to lightest. Each step up is "closer" (research: dark UIs show
// elevation with lighter surfaces; shadows vanish on dark).
/// The page behind everything.
pub const WINDOW: u32 = 0xff111111;
/// Navigation column, toolbars, status lines.
pub const SIDEBAR: u32 = 0xff161616;
/// A group of rows / content panel.
pub const PANEL: u32 = 0xff1b1b1b;
/// Controls at rest, the navigation's selection.
pub const RAISE: u32 = 0xff242424;
/// One step above `RAISE`: hover and keyboard focus.
pub const HOVER: u32 = 0xff2c2c2c;
/// The chosen thing: selected row, pressed control, a segment's choice.
pub const SELECTED: u32 = 0xff363636;
/// Dividers, only where space alone can't separate.
pub const LINE: u32 = 0xff262626;

// Text.
pub const TEXT: u32 = 0xffececec;
/// Secondary text: hints, column values, unselected navigation (6:1 on PANEL).
pub const SUB: u32 = 0xff9b9b9b;
/// Disabled only (no contrast promise).
pub const DIM: u32 = 0xff5e5e5e;
/// The muted sand accent: text caret and tiny highlights, rarely (ui skill §2).
pub const ACCENT: u32 = 0xffd9c8af;
/// Errors only (5:1 on WINDOW).
pub const ERROR: u32 = 0xffe06c6c;

/// Glass for app windows: `WINDOW` tint at 72 %. The bar's accepted 35 % leaves body
/// text at 2.9:1 over a white wallpaper (ui skill math: "check contrast on a bright
/// wallpaper"); 72 % with `sub` at 80 % was the lowest tried that keeps both ≥ 4.5 on every surface
/// (test `glass_text_passes_over_a_white_wallpaper`). The bar keeps its 35 %.
pub const GLASS_TINT: f32 = 0.72;

/// Spacing, 4-grid. Gap between groups ≥ 1.5× the gap inside one.
pub const S4: f32 = 4.0;
pub const S8: f32 = 8.0;
pub const S12: f32 = 12.0;
pub const S16: f32 = 16.0;
pub const S24: f32 = 24.0;
pub const S32: f32 = 32.0;

/// Controls, rows and navigation entries.
pub const RADIUS: f32 = 8.0;
/// A panel: its rows sit `S4` in, so `RADIUS` = `R_PANEL` − `S4` (concentric).
pub const R_PANEL: f32 = 12.0;

/// Font: Adwaita Sans (Inter-based, installed with GNOME's fonts): a UI face with
/// real Medium and SemiBold. Sizes in px; capitals land on whole px.
pub const FAMILY: &str = "Adwaita Sans";
/// Page title (SemiBold).
pub const TITLE: f32 = 20.0;
/// Rows, controls, navigation (Medium for controls, Regular for content).
pub const BODY: f32 = 14.0;
/// Group headings (SemiBold, `SUB`) and hints.
pub const SMALL: f32 = 12.0;

/// Every control and navigation entry is this tall.
pub const CONTROL_H: f32 = 32.0;
/// A dense content row (file lists).
pub const ROW_H: f32 = 28.0;
/// Icon beside text, and alone.
pub const ICON: f32 = 16.0;
/// Switch track and knob.
pub const SWITCH_W: f32 = 36.0;
pub const SWITCH_H: f32 = 20.0;
pub const KNOB: f32 = 14.0;
/// Scroll thumb width.
pub const THUMB_W: f32 = 4.0;
/// One mouse-wheel notch, px.
pub const WHEEL_STEP: f32 = 48.0;

/// Motion (ui skill §3, accepted): springs ζ = 1, ω = 2π / duration.
/// SNAP: hover, press, selection. GLIDE: layout, scrolling.
pub const SNAP: f32 = 0.15;
pub const GLIDE: f32 = 0.22;
/// Drawers (bar): 500 ms on the graphic curve.
pub const DRAWER: f32 = 0.5;
pub const DRAWER_CURVE: (f32, f32, f32, f32) = (0.76, 0.0, 0.12, 1.0);

/// Premultiplied white at `a` (0-1): the glass grays.
pub fn white(a: f32) -> u32 {
    let v = (a.clamp(0.0, 1.0) * 255.0).round() as u32;
    v << 24 | v << 16 | v << 8 | v
}

/// An opaque `rgb` color at alpha `a`, premultiplied.
pub fn alpha(rgb: u32, a: f32) -> u32 {
    let a = a.clamp(0.0, 1.0);
    let ch = |s: u32| (((rgb >> s & 0xff) as f32 * a).round() as u32) << s;
    ((a * 255.0).round() as u32) << 24 | ch(16) | ch(8) | ch(0)
}

/// The surfaces widgets use, solid or glass (glass: white at alpha over the tint,
/// ui skill §2). Text colors are the same in both.
#[derive(Clone, Copy)]
pub struct Palette {
    pub window: u32,
    pub sidebar: u32,
    pub panel: u32,
    pub raise: u32,
    pub hover: u32,
    pub selected: u32,
    pub line: u32,
    pub text: u32,
    pub sub: u32,
    pub dim: u32,
}

impl Palette {
    pub fn new(glass: bool) -> Self {
        if glass {
            Palette {
                window: alpha(WINDOW, GLASS_TINT),
                sidebar: white(0.02),
                panel: white(0.03),
                raise: white(0.05),
                hover: white(0.07),
                selected: white(0.10),
                line: white(0.08),
                text: TEXT,
                sub: alpha(TEXT, 0.8),
                dim: alpha(TEXT, 0.35),
            }
        } else {
            Palette {
                window: WINDOW,
                sidebar: SIDEBAR,
                panel: PANEL,
                raise: RAISE,
                hover: HOVER,
                selected: SELECTED,
                line: LINE,
                text: TEXT,
                sub: SUB,
                dim: DIM,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of an opaque gray-ish color.
    fn lum(c: u32) -> f64 {
        let lin = |v: u32| {
            let s = f64::from(v & 0xff) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c >> 16) + 0.7152 * lin(c >> 8) + 0.0722 * lin(c)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_passes_wcag_on_every_surface_it_sits_on() {
        for bg in [WINDOW, SIDEBAR, PANEL, RAISE, HOVER, SELECTED] {
            assert!(contrast(TEXT, bg) >= 4.5, "TEXT on {bg:08x}");
        }
        // SUB never sits on SELECTED: a selected row lifts its secondary text.
        for bg in [WINDOW, SIDEBAR, PANEL, RAISE, HOVER] {
            let c = contrast(SUB, bg);
            assert!(c >= 4.5, "SUB on {bg:08x}: {c}");
        }
        assert!(contrast(ERROR, WINDOW) >= 4.5);
        // Inverted emphasis: dark text on the light fill.
        assert!(contrast(WINDOW, TEXT) >= 4.5);
    }

    #[test]
    fn surfaces_step_up() {
        let s = [WINDOW, SIDEBAR, PANEL, RAISE, HOVER, SELECTED];
        assert!(s.windows(2).all(|w| lum(w[0]) < lum(w[1])));
    }
}

#[cfg(test)]
mod glass_tests {
    use super::*;
    use crate::ui::paint::over;

    fn lum(c: u32) -> f64 {
        let lin = |v: u32| {
            let s = f64::from(v & 0xff) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c >> 16) + 0.7152 * lin(c >> 8) + 0.0722 * lin(c)
    }

    /// Contrast of `ink` drawn over `bg` (both may be translucent; bg is opaque here).
    fn contrast(ink: u32, bg: u32) -> f64 {
        let fg = over(bg, ink, 255);
        let (x, y) = (lum(fg), lum(bg));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Glass over the worst wallpaper: pure white after Hyprland's blur brightness 0.8.
    /// Text sits on the window, the sidebar, panels, and rows/controls on panels.
    #[test]
    fn glass_text_passes_over_a_white_wallpaper() {
        let pal = Palette::new(true);
        let window = over(0xffcc_cccc, pal.window, 255);
        let panel = over(window, pal.panel, 255);
        let surfaces = [
            ("window", window, true),
            ("sidebar", over(window, pal.sidebar, 255), true),
            ("panel", panel, true),
            ("raise", over(panel, pal.raise, 255), true),
            ("hover", over(panel, pal.hover, 255), true),
            ("selected", over(panel, pal.selected, 255), false),
        ];
        for (name, bg, sub_too) in surfaces {
            let t = contrast(pal.text, bg);
            assert!(t >= 4.5, "text on glass {name}: {t:.2}");
            if sub_too {
                let s = contrast(pal.sub, bg);
                assert!(s >= 4.5, "sub on glass {name}: {s:.2}");
            }
        }
    }
}
