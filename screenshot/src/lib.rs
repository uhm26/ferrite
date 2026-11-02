//! screenshot: a fast Wayland screenshot tool. 0 crates.
//!
//! capture (wlr-screencopy into shm) → compose (outputs into one RGB canvas, row by
//! row) → png (own encoder: stripes filtered and deflated on all cores) → file/stdout.

pub mod app;
pub mod capture;
pub mod clipboard;
pub mod compose;
pub mod png;
pub mod select;
pub mod sys;
pub mod wl;
