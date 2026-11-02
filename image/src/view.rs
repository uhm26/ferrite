//! Where the image sits in the window: zoom and pan, and painting that.
//! Fit (the default) shows the whole image as big as the window allows and refits
//! on resize. Zooming glides (a spring on the zoom), keeps the point under the
//! pointer still and never goes below fit, so zoomed out is always "the whole picture".
//! A turn spins: the already turned picture is drawn turned back by what is left to go.

use std::time::Instant;

use frame::ui::motion::Spring;
use frame::ui::theme::{GLIDE, SNAP, WINDOW};
use pix::Mip;

/// Pixel peeping stops at 32×.
const MAX_ZOOM: f64 = 32.0;
/// From 4× on, pixels show as crisp squares instead of blurring together.
const SHARP_ZOOM: f64 = 4.0;
/// One wheel click zooms by this factor.
pub const STEP: f64 = 1.25;

/// Largest window [`snug`] asks for. shortcut: 75 % of the 1920×1080 monitors here;
/// upgrade: xdg_toplevel configure_bounds (xdg_wm_base v4) in frame.
const MAX_W: f64 = 1440.0;
const MAX_H: f64 = 810.0;
/// Tiny images still get a usable window: the long side grows to this.
const MIN_SIDE: f64 = 320.0;
/// Fit crops up to this many px instead of leaving a bar: the window's shape
/// matches the image's only to the nearest px.
const SLACK: f64 = 2.0;
/// The backdrop is the picture shrunk to about this many px on its long side, then
/// blown up: the blur. Dimmed to this many 256ths.
const BLUR_PX: f64 = 16.0;
const BLUR_DIM: u32 = 100;

pub struct View {
    pub zoom: f64,
    /// Follow the window: refit on every layout.
    fit: bool,
    /// The fit zoom at the last layout: the floor for zooming out.
    min: f64,
    /// The image's top left in window px; NaN until the first layout centers it.
    x: f64,
    y: f64,
    /// ln(zoom) gliding to the zoom asked for (log: every wheel click feels the same).
    glide: Spring,
    /// The zoom asked for, exact (the f32 spring only carries the way there).
    goal: f64,
    /// The window point that stays put while the zoom glides.
    at: (f64, f64),
    /// Degrees of a turn still to go (clockwise > 0), easing to 0.
    turn: Spring,
    /// When the glide or turn last moved.
    last: Instant,
}

impl View {
    pub fn new() -> Self {
        View {
            zoom: 1.0,
            fit: true,
            min: 0.0,
            x: f64::NAN,
            y: f64::NAN,
            glide: Spring::new(0.0, SNAP),
            goal: 1.0,
            at: (0.0, 0.0),
            turn: Spring::new(0.0, GLIDE),
            last: Instant::now(),
        }
    }

    /// Degrees of a turn still to go (0 when still).
    pub fn turn_left(&self) -> f32 {
        self.turn.value
    }

    /// A view of a picture just turned by `deg` (90 = a quarter clockwise): it spins
    /// there from where it was.
    pub fn turned(mut self, deg: f32) -> Self {
        self.turn = Spring::new(-deg, GLIDE);
        self.turn.set(0.0);
        self
    }

    /// Glide back to fit, `at` (the window's middle) staying put.
    pub fn fit(&mut self, at: (f64, f64)) {
        self.zoom_at(0.0, at);
    }

    /// Glide to the zoom asked for times `factor`, keeping window point `at` over the
    /// same image pixel. Reaching the fit zoom goes back to fit (follows the window).
    pub fn zoom_at(&mut self, factor: f64, at: (f64, f64)) {
        let goal = self.goal * factor;
        let goal = goal.clamp(self.min.clamp(1e-6, MAX_ZOOM), MAX_ZOOM);
        if !self.moving() {
            self.last = Instant::now();
        }
        (self.fit, self.at, self.goal) = (goal <= self.min, at, goal);
        self.glide.set(goal.ln() as f32);
    }

    /// 1:1 around window point `at`.
    pub fn actual(&mut self, at: (f64, f64)) {
        self.zoom_at(1.0 / self.goal, at);
    }

    /// Still gliding or turning: keep drawing frames.
    pub fn moving(&self) -> bool {
        self.glide.moving() || self.turn.moving()
    }

    /// Move the glide and turn on to `now`. True if anything changed.
    pub fn animate(&mut self, now: Instant) -> bool {
        if !self.moving() {
            return false;
        }
        let dt = now.saturating_duration_since(self.last).as_secs_f32();
        self.last = now;
        self.turn.step(dt);
        if !self.glide.moving() {
            return true;
        }
        let z = if self.glide.step(dt) {
            f64::from(self.glide.value).exp()
        } else {
            self.goal // arrived: exactly, so 1:1 is whole pixels
        };
        let k = z / self.zoom;
        self.x = self.at.0 - (self.at.0 - self.x) * k;
        self.y = self.at.1 - (self.at.1 - self.y) * k;
        self.zoom = z;
        true
    }

    pub fn pan(&mut self, dx: f64, dy: f64) {
        (self.x, self.y) = (self.x + dx, self.y + dy);
    }

    /// Settle zoom and position for an `img` (w, h) in a `win` (w, h): fit if fitting;
    /// a side smaller than the window is centered, a larger one can't leave a gap.
    pub fn layout(&mut self, img: (usize, usize), win: (usize, usize)) {
        let (iw, ih) = (img.0 as f64, img.1 as f64);
        let (ww, wh) = (win.0 as f64, win.1 as f64);
        let (contain, cover) = ((ww / iw).min(wh / ih), (ww / iw).max(wh / ih));
        let close = iw * cover - ww <= SLACK && ih * cover - wh <= SLACK;
        self.min = if close { cover } else { contain };
        if self.fit && !self.glide.moving() {
            (self.zoom, self.goal) = (self.min, self.min);
            self.glide.set(self.min.ln() as f32);
            self.glide.finish();
        }
        let place = |pos: f64, size: f64, room: f64| {
            if size <= room || pos.is_nan() {
                ((room - size) / 2.0).round()
            } else {
                pos.clamp(room - size, 0.0)
            }
        };
        self.x = place(self.x, iw * self.zoom, ww);
        self.y = place(self.y, ih * self.zoom, wh);
    }

    /// Fill `px` (`w` × `h`, the window) with the visible part of `mip`'s image, laid
    /// out first, and any room around it with [`backdrop`].
    pub fn paint(&mut self, mip: &Mip, (px, w, h): (&mut [u32], usize, usize)) {
        let img = mip.full();
        self.layout((img.w, img.h), (w, h));
        if self.turn.moving() {
            return self.paint_turn(mip, (px, w, h));
        }
        let (ox, oy) = (self.x.round(), self.y.round());
        let x0 = ox.max(0.0) as usize;
        let y0 = oy.max(0.0) as usize;
        let x1 = (ox + (img.w as f64 * self.zoom).round()).min(w as f64) as usize;
        let y1 = (oy + (img.h as f64 * self.zoom).round()).min(h as f64) as usize;
        if x0 > 0 || y0 > 0 || x1 < w || y1 < h {
            backdrop(mip, (px, w, h));
        }
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        // Crop in level-0 px, then in the picked level's px (`f` times smaller).
        let crop = ((x0 as f64 - ox) / self.zoom, (y0 as f64 - oy) / self.zoom);
        let (lvl, f) = mip.pick(self.zoom);
        let dst = (&mut px[y0 * w + x0..], w);
        let draw = if self.zoom >= SHARP_ZOOM {
            pix::nearest
        } else {
            pix::resample
        };
        draw(
            lvl,
            self.zoom * f,
            (crop.0 / f, crop.1 / f),
            (x1 - x0, y1 - y0),
            dst,
        );
    }

    /// Mid-turn: the picture (and its backdrop) spun back by the degrees left, centered,
    /// its size going from the old fit (sides swapped) to the new one.
    fn paint_turn(&self, mip: &Mip, (px, w, h): (&mut [u32], usize, usize)) {
        let img = mip.full();
        let (iw, ih, ww, wh) = (img.w as f64, img.h as f64, w as f64, h as f64);
        let left = f64::from(self.turn.value);
        let t = (left / 90.0).abs().min(1.0);
        let before = (ww / ih).min(wh / iw);
        let s = (self.zoom.ln() * (1.0 - t) + before.ln() * t).exp();
        let a = left.to_radians();
        spun_backdrop(mip, a, (px, w, h));
        let (lvl, f) = mip.pick(s);
        pix::transform(lvl, spin(f, s, a, (iw, ih, ww, wh)), (w, h), (px, w));
    }
}

/// The map from window px to px of a mip level `f` times smaller for a picture
/// (`iw` × `ih`) at `scale`, turned by `a` radians clockwise, centered in the window.
fn spin(f: f64, scale: f64, a: f64, (iw, ih, ww, wh): (f64, f64, f64, f64)) -> [f64; 6] {
    // Window px − window center, turned back by `a`, unscaled, + picture center.
    let (c, s) = (a.cos() / (scale * f), a.sin() / (scale * f));
    let (cx, cy, ox, oy) = (ww / 2.0, wh / 2.0, iw / 2.0 / f, ih / 2.0 / f);
    [c, s, ox - c * cx - s * cy, -s, c, oy + s * cx - c * cy]
}

/// The bars around a picture that doesn't fill the window: the picture itself, blown up
/// from a 16–32 px mip level (stretching that far is the blur, ~free) and dimmed so the
/// real picture stands out.
fn backdrop(mip: &Mip, (px, w, h): (&mut [u32], usize, usize)) {
    let full = mip.full();
    let (tiny, _) = mip.pick(BLUR_PX / full.w.max(full.h).max(1) as f64);
    pix::cover(tiny, w, h, px);
    dim(px);
}

/// [`backdrop`] turned by `a` radians: spun at a quarter of the window's size (it is a
/// blur: full size cost 3 ms of a 6 ms frame), then blown up.
fn spun_backdrop(mip: &Mip, a: f64, (px, w, h): (&mut [u32], usize, usize)) {
    const Q: usize = 4;
    let full = mip.full();
    let (iw, ih) = (full.w as f64, full.h as f64);
    let (bw, bh) = (w.div_ceil(Q), h.div_ceil(Q));
    let (sw, sh) = (bw as f64, bh as f64);
    // Scale that covers the small window's box turned into the picture's frame.
    let (c, s) = (a.cos().abs(), a.sin().abs());
    let cover = ((sw * c + sh * s) / iw).max((sw * s + sh * c) / ih);
    let (tiny, f) = mip.pick(BLUR_PX / iw.max(ih));
    let mut small = vec![WINDOW; bw * bh]; // edge px a float might miss
    pix::transform(
        tiny,
        spin(f, cover, a, (iw, ih, sw, sh)),
        (bw, bh),
        (&mut small, bw),
    );
    dim(&mut small);
    let rgb = small
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    let small = pix::Rgb {
        w: bw,
        h: bh,
        px: rgb.collect(),
    };
    pix::resample(&small, Q as f64, (0.0, 0.0), (w, h), (px, w));
}

fn dim(px: &mut [u32]) {
    for p in px.iter_mut() {
        let dim = |c: u32| ((c & 0xff) * BLUR_DIM / 256) & 0xff;
        *p = 0xff00_0000 | dim(*p >> 16) << 16 | dim(*p >> 8) << 8 | dim(*p);
    }
}

/// The window size that shows an image of `size` whole at its shape: 1:1 when it
/// fits [`MAX_W`] × [`MAX_H`], else shrunk to; tiny ones grown to [`MIN_SIDE`].
pub fn snug((iw, ih): (usize, usize)) -> (usize, usize) {
    let (iw, ih) = (iw as f64, ih as f64);
    let s = (MIN_SIDE / iw.max(ih))
        .max(1.0)
        .min(MAX_W / iw)
        .min(MAX_H / ih);
    let px = |v: f64| (v * s).round().max(1.0) as usize;
    (px(iw), px(ih))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pix::Rgb;

    #[test]
    fn snug_keeps_the_shape_within_limits() {
        assert_eq!(snug((800, 600)), (800, 600)); // fits: 1:1
        assert_eq!(snug((4733, 2662)), (1440, 810)); // shrunk to the max
        assert_eq!(snug((600, 3000)), (162, 810)); // tall: height bound
        assert_eq!(snug((16, 8)), (320, 160)); // tiny: grown
    }

    #[test]
    fn fit_fills_the_window_and_centers() {
        let mut v = View::new();
        v.layout((4000, 2000), (1000, 1000));
        assert_eq!((v.zoom, v.x, v.y), (0.25, 0.0, 250.0));
        v.layout((100, 50), (1000, 1000)); // small images grow too: no bars
        assert_eq!((v.zoom, v.x, v.y), (10.0, 0.0, 250.0));
        v.layout((1001, 500), (500, 250)); // shape off by half a px: crop, no bar
        assert_eq!((v.zoom, v.x, v.y), (0.5, 0.0, 0.0));
    }

    /// Let the zoom glide finish.
    fn settle(v: &mut View) {
        v.animate(v.last + std::time::Duration::from_secs(1));
        assert!(!v.moving());
    }

    #[test]
    fn zoom_glides_and_keeps_the_pointed_pixel_and_pan_stays_in_bounds() {
        let mut v = View::new();
        v.layout((1000, 1000), (500, 500)); // zoom 0.5, at (0, 0)
        v.zoom_at(2.0, (100.0, 100.0)); // 1:1, pixel 200 stays at 100
        v.animate(v.last + std::time::Duration::from_secs_f32(SNAP / 4.0));
        assert!(v.zoom > 0.5 && v.zoom < 1.0 && v.moving(), "{}", v.zoom);
        settle(&mut v);
        v.layout((1000, 1000), (500, 500));
        assert_eq!((v.zoom, v.x, v.y), (1.0, -100.0, -100.0));
        v.pan(1000.0, -9000.0);
        v.layout((1000, 1000), (500, 500));
        assert_eq!((v.x, v.y), (0.0, -500.0));
        v.zoom_at(0.01, (0.0, 0.0)); // can't go below fit, and fits again
        settle(&mut v);
        v.layout((1000, 1000), (500, 500));
        assert_eq!((v.zoom, v.fit, v.x, v.y), (0.5, true, 0.0, 0.0));
    }

    #[test]
    fn high_zoom_shows_crisp_pixels() {
        let mut img = Rgb {
            w: 2,
            h: 1,
            px: vec![0; 6],
        };
        img.px[3..].copy_from_slice(&[255; 3]);
        let mut v = View::new();
        v.layout((2, 1), (8, 4)); // fit: 4×, exactly SHARP_ZOOM
        let mut px = vec![0; 8 * 4];
        v.paint(&Mip::new(img), (&mut px, 8, 4));
        assert_eq!(
            px[..8],
            [
                0xff00_0000,
                0xff00_0000,
                0xff00_0000,
                0xff00_0000,
                !0,
                !0,
                !0,
                !0
            ]
        );
    }

    #[test]
    fn paint_fills_the_middle() {
        let img = Rgb {
            w: 2,
            h: 2,
            px: vec![255; 12],
        };
        let mut px = vec![0; 12 * 8];
        View::new().paint(&Mip::new(img), (&mut px, 12, 8));
        let white = |x: usize, y: usize| px[y * 12 + x] == 0xffff_ffff;
        assert!(white(2, 0) && white(9, 7)); // 2×2 grown to 8×8 at x 2
        assert!(!white(1, 0) && !white(10, 7) && px[0] == 0xff63_6363); // dimmed backdrop
    }

    #[test]
    fn a_turn_starts_where_the_picture_was() {
        // Black | white turned clockwise is 1×2, black on top. At the start of the
        // spin it must still show side by side: black left, white right.
        let img = Rgb {
            w: 1,
            h: 2,
            px: vec![0, 0, 0, 255, 255, 255],
        };
        let mut v = View::new().turned(90.0);
        let mut px = vec![0; 8 * 8];
        v.paint(&Mip::new(img), (&mut px, 8, 8));
        assert_eq!((px[4 * 8 + 1], px[4 * 8 + 6]), (0xff00_0000, 0xffff_ffff));
        v.animate(v.last + std::time::Duration::from_secs(1));
        assert!(!v.moving() && v.turn_left() == 0.0);
    }
}
