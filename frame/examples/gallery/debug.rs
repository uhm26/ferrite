//! The debug overlay (F12): frames per second, draw time, memory, window size,
//! springs, and whether the loop is idle. Numbers are for the last second.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use frame::ui::theme::*;
use frame::ui::{Align, Rect, Ui};

#[derive(Default)]
pub struct Stats {
    /// When each frame of the last second was drawn and how long drawing took.
    frames: VecDeque<(Instant, Duration)>,
    pub animating: bool,
}

impl Stats {
    pub fn frame(&mut self, at: Instant, took: Duration) {
        self.frames.push_back((at, took));
        while self
            .frames
            .front()
            .is_some_and(|(t, _)| at - *t > Duration::from_secs(1))
        {
            self.frames.pop_front();
        }
    }

    /// Mean and max draw time over the kept frames.
    pub fn summary(&self) -> String {
        let ms: Vec<f64> = self
            .frames
            .iter()
            .map(|f| f.1.as_secs_f64() * 1e3)
            .collect();
        let mean = ms.iter().sum::<f64>() / ms.len().max(1) as f64;
        let max = ms.iter().copied().fold(0.0, f64::max);
        format!(
            "draw mean {mean:.2} ms, max {max:.2} ms over {} frames",
            ms.len()
        )
    }

    fn lines(&self, ui: &Ui) -> [String; 5] {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let last = self.frames.back().map_or(0.0, |f| ms(f.1));
        let max = self.frames.iter().map(|f| ms(f.1)).fold(0.0, f64::max);
        let (w, h) = ui.p.size();
        let state = if self.animating {
            "animating"
        } else {
            "idle (0 CPU)"
        };
        [
            format!("fps    {}", self.frames.len()),
            format!("draw   {last:.2} ms, max {max:.2}"),
            format!("rss    {:.1} MB", rss_mb()),
            format!("size   {w}×{h}"),
            format!("state  {state}, {} springs", ui.s.springs()),
        ]
    }

    /// The overlay, top right: a raised flat surface, small text.
    pub fn draw(&self, ui: &mut Ui, w: f32) {
        let lines = self.lines(ui);
        let r = Rect::new(w - 260.0 - S16, S16, 260.0, 5.0 * 18.0 + 2.0 * S12).round();
        let fill = ui.pal.raise;
        ui.p.rect(r, R_PANEL, fill);
        let ink = ui.small().color(ui.pal.text);
        for (i, l) in lines.iter().enumerate() {
            let row = Rect::new(
                r.x + S12,
                r.y + S12 + i as f32 * 18.0,
                r.w - 2.0 * S12,
                18.0,
            );
            ui.text(row, l, ink, Align::Left);
        }
    }
}

/// Resident memory from /proc/self/statm (pages of 4 KiB).
fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: f64 = s
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    pages * 4096.0 / 1e6
}
