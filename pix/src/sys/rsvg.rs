//! librsvg (2.63) + the cairo and GLib calls it needs: /usr/include/librsvg-2.0/librsvg/
//! rsvg.h + rsvg-cairo.h, /usr/include/cairo/cairo.h, glib/gerror.h.
#![allow(non_camel_case_types)] // C type names kept as in the headers

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct RsvgHandle {
    _p: [u8; 0],
}
#[repr(C)]
pub struct cairo_t {
    _p: [u8; 0],
}
#[repr(C)]
pub struct cairo_surface_t {
    _p: [u8; 0],
}

/// gerror.h: `struct _GError { GQuark domain; gint code; gchar *message; }`
#[repr(C)]
pub struct GError {
    pub domain: u32,
    pub code: c_int,
    pub message: *const c_char,
}

/// rsvg.h: `struct _RsvgRectangle { double x, y, width, height; }`
#[repr(C)]
#[derive(Default)]
pub struct RsvgRectangle {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// rsvg.h: `typedef struct { double length; RsvgUnit unit; } RsvgLength;`
#[repr(C)]
#[derive(Default)]
pub struct RsvgLength {
    pub length: f64,
    pub unit: c_int,
}

/// cairo.h: `CAIRO_FORMAT_ARGB32 = 0` (premultiplied, native-endian u32).
pub const CAIRO_FORMAT_ARGB32: c_int = 0;
/// cairo.h: `CAIRO_STATUS_SUCCESS = 0`.
pub const CAIRO_STATUS_SUCCESS: c_int = 0;

#[link(name = "rsvg-2")]
unsafe extern "C" {
    // RsvgHandle *rsvg_handle_new_from_data (const guint8 *data, gsize data_len, GError **error);
    pub fn rsvg_handle_new_from_data(d: *const u8, n: usize, e: *mut *mut GError) -> *mut RsvgHandle;
    // gboolean rsvg_handle_get_intrinsic_size_in_pixels (RsvgHandle *, gdouble *, gdouble *);
    pub fn rsvg_handle_get_intrinsic_size_in_pixels(h: *mut RsvgHandle, w: *mut f64, ht: *mut f64) -> c_int;
    // void rsvg_handle_get_intrinsic_dimensions (RsvgHandle *, gboolean *, RsvgLength *,
    //     gboolean *, RsvgLength *, gboolean *, RsvgRectangle *);
    pub fn rsvg_handle_get_intrinsic_dimensions(
        h: *mut RsvgHandle,
        has_w: *mut c_int,
        w: *mut RsvgLength,
        has_h: *mut c_int,
        ht: *mut RsvgLength,
        has_vb: *mut c_int,
        vb: *mut RsvgRectangle,
    );
    // gboolean rsvg_handle_render_document (RsvgHandle *, cairo_t *, const RsvgRectangle *, GError **);
    pub fn rsvg_handle_render_document(
        h: *mut RsvgHandle,
        cr: *mut cairo_t,
        viewport: *const RsvgRectangle,
        e: *mut *mut GError,
    ) -> c_int;
}

#[link(name = "cairo")]
unsafe extern "C" {
    // cairo_surface_t *cairo_image_surface_create (cairo_format_t, int width, int height);
    pub fn cairo_image_surface_create(f: c_int, w: c_int, h: c_int) -> *mut cairo_surface_t;
    // cairo_status_t cairo_surface_status (cairo_surface_t *surface);
    pub fn cairo_surface_status(s: *mut cairo_surface_t) -> c_int;
    // void cairo_surface_flush (cairo_surface_t *surface);
    pub fn cairo_surface_flush(s: *mut cairo_surface_t);
    // unsigned char *cairo_image_surface_get_data (cairo_surface_t *surface);
    pub fn cairo_image_surface_get_data(s: *mut cairo_surface_t) -> *mut u8;
    // int cairo_image_surface_get_stride (cairo_surface_t *surface);
    pub fn cairo_image_surface_get_stride(s: *mut cairo_surface_t) -> c_int;
    // void cairo_surface_destroy (cairo_surface_t *surface);
    pub fn cairo_surface_destroy(s: *mut cairo_surface_t);
    // cairo_t *cairo_create (cairo_surface_t *target);
    pub fn cairo_create(s: *mut cairo_surface_t) -> *mut cairo_t;
    // void cairo_destroy (cairo_t *cr);
    pub fn cairo_destroy(cr: *mut cairo_t);
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    // void g_object_unref (gpointer object);
    pub fn g_object_unref(o: *mut c_void);
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    // void g_error_free (GError *error);
    pub fn g_error_free(e: *mut GError);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structs_match_the_header_sizes() {
        assert_eq!(size_of::<GError>(), 16);
        assert_eq!(size_of::<RsvgRectangle>(), 32);
        assert_eq!(size_of::<RsvgLength>(), 16);
    }
}
