//! frame — our own Wayland base for Rust apps, 0 crates.
//!
//! Planned modules (merged from term, wall, snap, clip; see docs/plans/core):
//! wire protocol, window + shm buffers, input + xkb, FreeType font, inotify.
//! Consumers copy a snapshot into `vendor/frame` (rule in ../AGENTS.md).
