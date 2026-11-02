//! Raw bindings to the system C libraries we call ourselves (rust-deps skill): only
//! what pix uses, signatures copied from the headers. Safe wrappers: `webp.rs`, `svg.rs`.

#[cfg(feature = "svg")]
pub mod rsvg;
#[cfg(feature = "webp")]
pub mod webp;
