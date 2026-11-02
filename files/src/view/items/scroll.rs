//! Scrolling a folder view: the wheel moves it freely, keys keep the selection in
//! sight, a spring eases the offset (GLIDE).

use frame::ui::Ui;
use frame::ui::theme::{GLIDE, WHEEL_STEP};

#[derive(Default)]
pub struct Scroll {
    /// Wanted offset, px.
    want: f32,
    /// Keep the selection in sight (after a key or an open, not after the wheel).
    follow: bool,
}

impl Scroll {
    pub fn wheel(&mut self, discrete: i32, px: f64) {
        self.want += if discrete != 0 {
            discrete as f32 * WHEEL_STEP
        } else {
            px as f32
        };
        self.follow = false;
    }

    pub fn follow(&mut self) {
        self.follow = true;
    }

    /// This frame's offset over `content` px shown in `view` px, with the selection
    /// spanning `sel` (top, height) in content px. `key` names the spring: one per
    /// folder, so entering another starts at its own offset instead of gliding.
    pub fn offset(
        &mut self,
        ui: &mut Ui,
        key: &str,
        sel: (f32, f32),
        (view, content): (f32, f32),
    ) -> f32 {
        if self.follow {
            // max then min, not clamp: a view shorter than a row would make lo > hi.
            self.want = self.want.max(sel.0 + sel.1 - view).min(sel.0);
        }
        self.want = self.want.clamp(0.0, (content - view).max(0.0));
        // shortcut: one spring per folder visited lives until exit, fine for a session.
        ui.spring(key, self.want, GLIDE).round()
    }
}
