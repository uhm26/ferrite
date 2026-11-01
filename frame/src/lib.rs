//! frame — our own Wayland base for Rust apps, 0 crates.
//!
//! [`window`] is one xdg toplevel with shm buffers, keyboard ([`xkb`]), pointer and
//! clipboard; it speaks Wayland through [`wl`]. [`font`] loads FreeType faces with
//! fontconfig fallback. [`ui`] is the look and the widgets on top. [`sys`] holds the
//! raw libc calls (poll, memfd, inotify). [`png`] writes headless screenshots.
//! Merged from term (most complete copy); see docs/plans/core.

pub mod font;
pub mod png;
pub mod sys;
pub mod ui;
pub mod window;
pub mod wl;
pub mod xkb;
