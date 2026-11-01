//! Where the image sits in the window: zoom and pan, and painting that.
//! Fit (the default) shows the whole image, never larger than 1:1, and refits on
//! resize. Zooming keeps the point under the pointer still.

use frame::ui::theme::WINDOW;

use crate::image::Rgb;
use crate::scale;

/// Zoom limits: a few screen px for a huge photo, pixel peeping at 32×.
const MIN_ZOOM: f64 = 0.01;
const MAX_ZOOM: f64 = 32.0;
/// One wheel click zooms by this factor.
pub const STEP: f64 = 1.25;

pub struct View {
    pub zoom: f64,
    /// Follow the window: refit on every layout.
    fit: bool,
    /// The image's top left in window px.
    x: f64,
    y: f64,
}

impl View {
    pub fn new() -> Self {
        View {
            zoom: 1.0,
            fit: true,
            x: 0.0,
            y: 0.0,
        }
    }

    pub fn fit(&mut self) {
        self.fit = true;
    }

    /// Zoom by `factor` keeping window point `at` over the same image pixel.
    pub fn zoom_at(&mut self, factor: f64, at: (f64, f64)) {
        let z = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.x = at.0 - (at.0 - self.x) * z / self.zoom;
        self.y = at.1 - (at.1 - self.y) * z / self.zoom;
        (self.zoom, self.fit) = (z, false);
    }

    /// 1:1 around window point `at`.
    pub fn actual(&mut self, at: (f64, f64)) {
        self.zoom_at(1.0 / self.zoom, at);
    }

    pub fn pan(&mut self, dx: f64, dy: f64) {
        (self.x, self.y) = (self.x + dx, self.y + dy);
    }

    /// Settle zoom and position for an `img` (w, h) in a `win` (w, h): fit if fitting;
    /// a side smaller than the window is centered, a larger one can't leave a gap.
    pub fn layout(&mut self, img: (usize, usize), win: (usize, usize)) {
        let (iw, ih) = (img.0 as f64, img.1 as f64);
        let (ww, wh) = (win.0 as f64, win.1 as f64);
        if self.fit {
            self.zoom = (ww / iw).min(wh / ih).min(1.0);
        }
        let place = |pos: f64, size: f64, room: f64| {
            if size <= room {
                ((room - size) / 2.0).round()
            } else {
                pos.clamp(room - size, 0.0)
            }
        };
        self.x = place(self.x, iw * self.zoom, ww);
        self.y = place(self.y, ih * self.zoom, wh);
    }

    /// Fill `px` (`w` × `h`, the window) with the window color and the visible part
    /// of `img`, laid out first. `scratch` keeps its allocation between frames.
    pub fn paint(
        &mut self,
        img: &Rgb,
        (px, w, h): (&mut [u32], usize, usize),
        scratch: &mut Vec<u32>,
    ) {
        self.layout((img.w, img.h), (w, h));
        px.fill(WINDOW);
        let (ox, oy) = (self.x.round(), self.y.round());
        let x0 = ox.max(0.0) as usize;
        let y0 = oy.max(0.0) as usize;
        let x1 = (ox + (img.w as f64 * self.zoom).round()).min(w as f64) as usize;
        let y1 = (oy + (img.h as f64 * self.zoom).round()).min(h as f64) as usize;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (dw, dh) = (x1 - x0, y1 - y0);
        scratch.resize(dw * dh, 0);
        let crop = ((x0 as f64 - ox) / self.zoom, (y0 as f64 - oy) / self.zoom);
        scale::resample(img, self.zoom, crop, (dw, dh), scratch);
        for (row, src) in scratch.chunks_exact(dw).enumerate() {
            let at = (y0 + row) * w + x0;
            px[at..at + dw].copy_from_slice(src);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_never_enlarges_and_centers() {
        let mut v = View::new();
        v.layout((4000, 2000), (1000, 1000));
        assert_eq!((v.zoom, v.x, v.y), (0.25, 0.0, 250.0));
        v.layout((100, 50), (1000, 1000));
        assert_eq!((v.zoom, v.x, v.y), (1.0, 450.0, 475.0));
    }

    #[test]
    fn zoom_keeps_the_pointed_pixel_and_pan_stays_in_bounds() {
        let mut v = View::new();
        v.layout((1000, 1000), (500, 500)); // zoom 0.5, at (0, 0)
        v.zoom_at(2.0, (100.0, 100.0)); // 1:1, pixel 200 stays at 100
        v.layout((1000, 1000), (500, 500));
        assert_eq!((v.zoom, v.x, v.y), (1.0, -100.0, -100.0));
        v.pan(1000.0, -9000.0);
        v.layout((1000, 1000), (500, 500));
        assert_eq!((v.x, v.y), (0.0, -500.0));
    }

    #[test]
    fn paint_puts_the_image_in_the_middle() {
        let img = Rgb {
            w: 2,
            h: 2,
            px: vec![255; 12],
        };
        let mut px = vec![0; 6 * 4];
        View::new().paint(&img, (&mut px, 6, 4), &mut Vec::new());
        let white = |x: usize, y: usize| px[y * 6 + x] == 0xffff_ffff;
        assert!(white(2, 1) && white(3, 2));
        assert!(!white(1, 1) && !white(4, 1) && !white(2, 0) && px[0] == WINDOW);
    }
}
