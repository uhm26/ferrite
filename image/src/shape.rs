//! The window's shape: it opens in the first picture's shape and then stays put (owner
//! 2026-11-02: no resizing on step or turn); pictures of other shapes get a blurred
//! backdrop around them (`view::backdrop`). Banners (wider or taller than
//! [`BANNER`]) open in a square instead, so turning them doesn't leave a thin strip.
//!
//! A client can't just draw another size (Hyprland sends its own back), so we lock
//! min = max = the size, wait for the configure that applies it, then unlock so the
//! owner can resize by hand. Limits apply on the next commit, so a lock asks for a frame.

use frame::window::Window;
use frame::wl::Error;

use crate::view::snug;

/// From this long side : short side on, a picture counts as a banner. Phone screenshots
/// (~2.2 : 1) still open in their own shape.
const BANNER: f64 = 2.5;

#[derive(Clone, Copy, PartialEq)]
enum State {
    /// Tiled, or already sized: the compositor or the owner picks the size.
    Free,
    /// Size the window to the first picture on the next loop turn.
    Want,
    Locked(usize, usize),
    /// The locked size arrived: lift the limits.
    Release,
}

pub struct Shape {
    state: State,
    /// The size of the last configure: locking to it again would wait forever.
    configured: (usize, usize),
}

impl Shape {
    /// `configured`: the first configure's size; (0, 0) = floating (we pick).
    pub fn new(configured: (usize, usize)) -> Self {
        let floating = configured == (0, 0);
        Shape {
            state: if floating { State::Want } else { State::Free },
            configured,
        }
    }

    /// A configure arrived with this size.
    pub fn resized(&mut self, w: usize, h: usize) {
        self.configured = (w, h);
        if self.state == State::Locked(w, h) {
            self.state = State::Release;
        }
    }

    /// Size the window for the first picture (`size`, None while none decoded). True if
    /// it sent size limits: draw a frame so they apply.
    pub fn turn(&mut self, win: &mut Window, size: Option<(usize, usize)>) -> Result<bool, Error> {
        match (self.state, size) {
            (State::Want, Some(size)) if window_for(size) == self.configured => {
                self.state = State::Free;
            }
            (State::Want, Some(size)) => {
                let (w, h) = window_for(size);
                win.set_size(w, h);
                win.set_size_limits((w, h), (w, h))?;
                self.state = State::Locked(w, h);
                return Ok(true);
            }
            (State::Want, None) => self.state = State::Free, // no image: keep the size
            (State::Release, _) => {
                win.set_size_limits((0, 0), (0, 0))?;
                self.state = State::Free;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}

/// The window a picture of `size` opens in: its own shape, a square for banners.
fn window_for((w, h): (usize, usize)) -> (usize, usize) {
    let long = w.max(h);
    if long as f64 >= BANNER * w.min(h).max(1) as f64 {
        snug((long, long))
    } else {
        snug((w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banners_open_square_the_rest_in_shape() {
        assert_eq!(window_for((1500, 500)), (810, 810)); // 3 : 1 banner
        assert_eq!(window_for((500, 1500)), (810, 810)); // turned banner
        assert_eq!(window_for((1080, 2340)), snug((1080, 2340))); // phone screenshot
        assert_eq!(window_for((1920, 1080)), (1440, 810));
    }
}
