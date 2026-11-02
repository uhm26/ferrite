//! pix — our own image library: decode PNG/JPEG/GIF/WebP into packed RGB frames, keep
//! SVG as a document to render at any size, build a mip chain, and resample into opaque
//! ARGB8888 buffers (zoom + crop, crisp nearest pixels, or cover fit); turn, and write PNG.
//!
//! ```ignore
//! let img = pix::load(path, [0x11; 3])?;    // first frame; transparency onto #111111
//! pix::cover(&img, w, h, &mut argb);         // wallpaper fit
//! match pix::open(path, bg)? {               // viewer: every frame, or the SVG itself
//!     pix::Image::Frames(f) => {}            // f[i].img, shown f[i].ms (0 = still)
//!     pix::Image::Svg(s) => { s.render((w, h), bg)?; }
//! }
//! let mip = pix::Mip::new(img);              // zoom without aliasing
//! let (level, k) = mip.pick(zoom);
//! pix::resample(level, zoom * k, crop, (w, h), (&mut argb, stride));
//! let up = pix::orient(&img, pix::CW);       // turn a quarter (JPEG EXIF is applied at load)
//! pix::transform(level, m, (w, h), (&mut argb, stride)); // any angle (affine, bilinear)
//! let bytes = pix::png(&img)?;              // PNG file bytes (clipboard)
//! ```
//! Codecs: zune (PNG, JPEG), the `gif` crate, system libwebp + librsvg through our own
//! bindings in [`sys`] (docs/plans/formats/2026-11-02-gif-webp-svg.md).

mod decode;
mod encode;
mod gif;
mod mip;
mod orient;
mod scale;
mod svg;
mod sys;
mod webp;

pub use decode::{Error, Frame, Image, Rgb, load, open};
pub use mip::Mip;
pub use encode::png;
pub use orient::{CCW, CW, orient};
pub use scale::{cover, nearest, resample, transform};
pub use svg::Svg;
