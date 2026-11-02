//! wallpaper — a tiny, fast Wayland wallpaper daemon.
//!
//! `wallpaper <image>` shows a PNG or JPEG on every monitor (wlr-layer-shell background
//! layer, cover fit). The first call becomes the daemon; later calls hand it the new
//! path over a unix socket and exit. No Wayland crates: we speak the wire protocol.
//!
//! - `app`   — the daemon: outputs, surfaces, drawing, event loop
//! - `wl`    — our Wayland client (wire format, opcodes, typed requests)
//! - `ipc`   — the one-daemon-per-session socket
//! - decode + cover fit come from `pix` (our image library, in this repo)
//! - `shm`   — shared pixel memory for the compositor
//! - `sys`   — the few libc calls std lacks (memfd, mmap, sendmsg, poll)

pub mod app;
pub mod ipc;
pub mod shm;
pub mod sys;
pub mod wl;
