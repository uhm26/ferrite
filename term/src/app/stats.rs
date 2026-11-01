//! Run statistics, printed on exit with `TERM_STATS=1`: how many frames, how long
//! drawing took, how much output was parsed, and memory (from /proc).

use std::time::{Duration, Instant};

pub(super) struct Stats {
    pub frames: u64,
    pub draw: Duration,
    pub draw_max: Duration,
    pub bytes: u64,
    start: Instant,
    /// Time from start to the first frame on screen.
    first: Option<Duration>,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            frames: 0,
            draw: Duration::ZERO,
            draw_max: Duration::ZERO,
            bytes: 0,
            start: Instant::now(),
            first: None,
        }
    }
}

impl Stats {
    pub fn add_frame(&mut self, took: Duration) {
        self.first.get_or_insert(self.start.elapsed());
        self.frames += 1;
        self.draw += took;
        self.draw_max = self.draw_max.max(took);
    }

    pub fn report(&self, extra: &str) -> String {
        let avg = self
            .draw
            .checked_div(self.frames as u32)
            .unwrap_or_default();
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let field = |name: &str| {
            status
                .lines()
                .find(|l| l.starts_with(name))
                .map(|l| l.split_whitespace().nth(1).unwrap_or("?").to_string())
                .unwrap_or_else(|| "?".into())
        };
        format!(
            "term stats: {:.2?} run, first frame at {:.2?}, {} frames, draw avg {avg:.2?} max {:.2?}, \
             {} KiB parsed, RSS {} KiB (peak {} KiB), {extra}",
            self.start.elapsed(),
            self.first.unwrap_or_default(),
            self.frames,
            self.draw_max,
            self.bytes / 1024,
            field("VmRSS:"),
            field("VmHWM:"),
        )
    }
}
