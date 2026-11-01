//! Parser + screen throughput without pty or window:
//! `cargo run --release --example bench`. Feeds generated output in 64 KiB chunks
//! (like the pty reader) and prints MB/s per workload.

use std::time::Instant;

use term::config::Config;
use term::term::Term;

fn run(name: &str, data: &[u8]) {
    run_with(name, data, Config::default().scrollback);
}

fn run_with(name: &str, data: &[u8], scrollback: usize) {
    let cfg = Config::default();
    let mut t = Term::new(200, 50, scrollback, cfg.palette());
    let start = Instant::now();
    for chunk in data.chunks(64 << 10) {
        t.feed(chunk);
        t.replies.clear();
    }
    let s = start.elapsed().as_secs_f64();
    let mb = data.len() as f64 / 1e6;
    println!(
        "{name:>10}: {mb:6.1} MB in {:6.1} ms = {:6.0} MB/s",
        s * 1e3,
        mb / s
    );
}

fn main() {
    term::sys::init_locale();
    let seq: String = (1..=3_000_000).map(|i| format!("{i}\n")).collect();
    run("seq", seq.replace('\n', "\r\n").as_bytes());
    run_with("seq no-sb", seq.replace('\n', "\r\n").as_bytes(), 0);
    let long = "the quick brown fox jumps over the lazy dog ".repeat(4);
    let text: String = (0..200_000).map(|_| format!("{long}\r\n")).collect();
    run("text", text.as_bytes());
    let colored: String = (0..200_000)
        .map(|i| {
            format!(
                "\x1b[3{}m{long}\x1b[0m\x1b[38;2;1;2;3mrgb\x1b[0m\r\n",
                i % 8
            )
        })
        .collect();
    run("sgr", colored.as_bytes());
    let utf8 = "héllo wörld ─│┌┐ λ→ ".repeat(8);
    let uni: String = (0..100_000).map(|_| format!("{utf8}\r\n")).collect();
    run("unicode", uni.as_bytes());
    // Resize cost with a full scrollback: reflow runs on every resize step.
    let cfg = Config::default();
    let mut t = Term::new(200, 50, cfg.scrollback, cfg.palette());
    t.feed(text.as_bytes());
    for cols in [120, 200, 199] {
        let start = Instant::now();
        t.resize(cols, 50);
        println!("{:>10}: {} lines to {cols} cols in {:.2} ms", "reflow", t.history_len(), start.elapsed().as_secs_f64() * 1e3);
    }
}
