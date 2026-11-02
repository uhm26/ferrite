//! Run statistics, printed on exit with `TERM_STATS=1`: how many frames, how long
//! drawing took, how much output was parsed, and memory (from /proc).

use std::time::{Duration, Instant};

pub(super) struct Stats {
    pub frames: u64,
    pub draw: Duration,
    pub draw_max: Duration,
    pub bytes: u64,
    start: Instant,
    /// `TERM_STATS` is set: only then are keypresses timed (the lists grow per key).
    on: bool,
    /// Time from start to the first frame on screen.
    first: Option<Duration>,
    /// Keypress latency, split: key → the app's first output (`app`), that output
    /// → the frame showing it (`show`, term's own share).
    key_at: Option<Instant>,
    out_at: Option<Instant>,
    app: Vec<Duration>,
    show: Vec<Duration>,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            frames: 0,
            draw: Duration::ZERO,
            draw_max: Duration::ZERO,
            bytes: 0,
            start: Instant::now(),
            on: std::env::var_os("TERM_STATS").is_some(),
            first: None,
            key_at: None,
            out_at: None,
            app: Vec::new(),
            show: Vec::new(),
        }
    }
}

impl Stats {
    pub fn key(&mut self) {
        if !self.on {
            return;
        }
        self.key_at.get_or_insert_with(Instant::now);
    }

    pub fn output(&mut self) {
        if self.key_at.is_some() {
            self.out_at.get_or_insert_with(Instant::now);
        }
    }

    /// A frame went to the compositor.
    pub fn presented(&mut self) {
        if let (Some(k), Some(o)) = (self.key_at, self.out_at.take()) {
            self.app.push(o - k);
            self.show.push(o.elapsed());
            self.key_at = None;
        }
    }

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
             {} KiB parsed, RSS {} KiB (peak {} KiB), {extra}\nkeys: {} answered; key→app output {}; output→on screen (term) {}",
            self.start.elapsed(),
            self.first.unwrap_or_default(),
            self.frames,
            self.draw_max,
            self.bytes / 1024,
            field("VmRSS:"),
            field("VmHWM:"),
            self.app.len(),
            spread(&self.app),
            spread(&self.show),
        )
    }
}

/// "median X, worst Y" of a list of durations.
fn spread(v: &[Duration]) -> String {
    let mut v = v.to_vec();
    v.sort();
    match (v.get(v.len() / 2), v.last()) {
        (Some(m), Some(w)) => format!("median {m:.1?}, worst {w:.1?}"),
        _ => "-".into(),
    }
}
