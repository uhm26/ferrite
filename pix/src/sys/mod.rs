//! Raw bindings to the system C libraries we call ourselves (rust-deps skill): only
//! what pix uses, signatures copied from the headers. Safe wrappers: `webp.rs`, `svg.rs`.

pub mod rsvg;
pub mod webp;
