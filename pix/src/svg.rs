//! SVG through the system librsvg (text via pango, 0 crates; resvg was 1.8× faster but
//! 55 crates). Vector: render at any size.

use std::ffi::CStr;

use crate::decode::{Error, Rgb};
use crate::sys::rsvg::*;

/// Largest side we render, px: keeps a hostile size from allocating gigabytes.
const MAX_SIDE: usize = 16384;
/// Size for an SVG that states none (only a percentage or nothing).
const FALLBACK: f64 = 512.0;

/// A parsed SVG document, ready to render at any size.
pub struct Svg(*mut RsvgHandle);

impl Drop for Svg {
    fn drop(&mut self) {
        // SAFETY: the handle came from rsvg_handle_new_from_data and is unref'd once.
        unsafe { g_object_unref(self.0.cast()) }
    }
}

/// Take ownership of a GError: its message as our error, freed.
fn gerror(e: *mut GError, what: &str) -> Error {
    if e.is_null() {
        return Error::Decode(format!("librsvg: {what} failed"));
    }
    // SAFETY: non-null GError from librsvg: message is a valid C string; freed once here.
    let msg = unsafe { CStr::from_ptr((*e).message) }.to_string_lossy().into_owned();
    // SAFETY: see above.
    unsafe { g_error_free(e) };
    Error::Decode(format!("librsvg: {msg}"))
}

impl Svg {
    pub fn new(bytes: &[u8]) -> Result<Self, Error> {
        let mut e = std::ptr::null_mut();
        // SAFETY: valid byte slice; librsvg copies what it needs before returning.
        let h = unsafe { rsvg_handle_new_from_data(bytes.as_ptr(), bytes.len(), &mut e) };
        if h.is_null() {
            return Err(gerror(e, "parse"));
        }
        Ok(Svg(h))
    }

    /// The document's own size in px: width/height, else its viewBox, else 512².
    pub fn size(&self) -> (f64, f64) {
        let (mut w, mut h) = (0.0, 0.0);
        // SAFETY: live handle, writable out-params.
        if unsafe { rsvg_handle_get_intrinsic_size_in_pixels(self.0, &mut w, &mut h) } != 0
            && w > 0.0
            && h > 0.0
        {
            return (w, h);
        }
        let (mut hw, mut hh, mut hv) = (0, 0, 0);
        let (mut lw, mut lh, mut vb) = Default::default();
        // SAFETY: live handle, every out-param points at a writable value of its type.
        unsafe {
            rsvg_handle_get_intrinsic_dimensions(
                self.0, &mut hw, &mut lw, &mut hh, &mut lh, &mut hv, &mut vb,
            );
        }
        let vb: RsvgRectangle = vb;
        if hv != 0 && vb.width > 0.0 && vb.height > 0.0 {
            (vb.width, vb.height)
        } else {
            (FALLBACK, FALLBACK)
        }
    }

    /// Render into `w` × `h` (aspect kept per the SVG's preserveAspectRatio),
    /// transparency onto `bg`.
    pub fn render(&self, (w, h): (usize, usize), bg: [u8; 3]) -> Result<Rgb, Error> {
        let (w, h) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
        // SAFETY: creating an image surface; its status is checked before use.
        let surf = unsafe { cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w as i32, h as i32) };
        let s = Surface(surf);
        // SAFETY: `surf` is a surface (maybe in an error state, checked here).
        if unsafe { cairo_surface_status(surf) } != CAIRO_STATUS_SUCCESS {
            return Err(Error::Decode("cairo: can't make the surface".into()));
        }
        let view = RsvgRectangle {
            width: w as f64,
            height: h as f64,
            ..Default::default()
        };
        let mut e = std::ptr::null_mut();
        // SAFETY: live surface; the context is destroyed right after rendering.
        let ok = unsafe {
            let cr = cairo_create(surf);
            let ok = rsvg_handle_render_document(self.0, cr, &view, &mut e);
            cairo_destroy(cr);
            cairo_surface_flush(surf);
            ok
        };
        if ok == 0 {
            return Err(gerror(e, "render"));
        }
        Ok(s.to_rgb((w, h), bg))
    }
}

/// Owns a cairo surface; destroys it on drop.
struct Surface(*mut cairo_surface_t);

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the surface came from cairo_image_surface_create, destroyed once.
        unsafe { cairo_surface_destroy(self.0) }
    }
}

impl Surface {
    /// Premultiplied ARGB32 (native-endian u32 rows `stride` apart) → RGB over `bg`.
    fn to_rgb(&self, (w, h): (usize, usize), bg: [u8; 3]) -> Rgb {
        // SAFETY: flushed image surface of w × h: data holds h rows of `stride` bytes.
        let (data, stride) = unsafe {
            (cairo_image_surface_get_data(self.0), cairo_image_surface_get_stride(self.0) as usize)
        };
        // SAFETY: see above.
        let all = unsafe { std::slice::from_raw_parts(data, stride * h) };
        let mut px = Vec::with_capacity(w * h * 3);
        for row in all.chunks_exact(stride) {
            for p in row[..w * 4].as_chunks::<4>().0 {
                let v = u32::from_ne_bytes(*p);
                let a = v >> 24;
                for (shift, b) in [16, 8, 0].into_iter().zip(bg) {
                    let c = (v >> shift) & 0xff; // already × alpha
                    px.push((c + (u32::from(b) * (255 - a) + 127) / 255).min(255) as u8);
                }
            }
        }
        Rgb { w, h, px }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: &[u8] = br#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="2" height="2" fill="red"/></svg>"#;

    #[test]
    fn renders_at_any_size_over_the_background() {
        let s = Svg::new(RED).unwrap();
        assert_eq!(s.size(), (4.0, 2.0));
        let img = s.render((8, 4), [0, 0, 9]).unwrap();
        assert_eq!((img.w, img.h), (8, 4));
        assert_eq!(img.px[..3], [255, 0, 0]); // left half: the rect
        assert_eq!(img.px[7 * 3..8 * 3], [0, 0, 9]); // right half: transparent → bg
        assert!(Svg::new(b"<svg").is_err());
    }
}
