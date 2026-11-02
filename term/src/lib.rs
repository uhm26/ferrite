//! term: a small, fast terminal emulator for Wayland.
//!
//! Data flow: shell → [`pty`] → [`vt`] parser → [`term`] (screen state) →
//! [`render`] (CPU, only changed cells) → shm buffer → [`window`] ([`wl`] protocol).
//! Keys come back through [`xkb`] and [`keys`]; [`app`] runs the event loop.

pub mod app;
pub mod color;
pub mod config;
pub mod font;
pub mod keys;
pub mod pty;
pub mod render;
pub mod sys;
pub mod term;
pub mod view;
pub mod vt;
pub mod window;
pub mod wl;
pub mod xkb;
