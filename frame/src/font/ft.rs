//! Own FreeType bindings: only what we use. Struct prefixes mirror
//! /usr/include/freetype2/freetype/freetype.h (FreeType 2.15, x86_64); the offsets
//! were measured with offsetof() and are pinned by the test below.
//! Copied into frame from ../term/src/font/ft.rs at term 7be66f5 (2026-11-01).

use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::path::Path;

type FtError = c_int;

/// FT_LOAD_RENDER: rasterize into an 8-bit coverage bitmap.
pub const LOAD_RENDER: i32 = 4;
/// FT_LOAD_TARGET_LIGHT: light vertical-only hinting (crisp, keeps shapes).
pub const LOAD_TARGET_LIGHT: i32 = 0x1_0000;
/// FT_PIXEL_MODE_GRAY
pub const PIXEL_MODE_GRAY: u8 = 2;

#[repr(C)]
pub struct LibraryRec {
    _p: [u8; 0],
}

/// freetype.h: FT_Vector (FT_Pos = signed long, 26.6 fixed point)
#[repr(C)]
pub struct Vector {
    pub x: c_long,
    pub y: c_long,
}

/// ftimage.h: FT_Bitmap
#[repr(C)]
pub struct Bitmap {
    pub rows: c_uint,
    pub width: c_uint,
    pub pitch: c_int,
    pub buffer: *mut u8,
    pub num_grays: u16,
    pub pixel_mode: u8,
    pub palette_mode: u8,
    pub palette: *mut u8,
}

/// FT_GlyphSlotRec up to bitmap_top (freetype.h: library, face, next, glyph_index,
/// generic, metrics come first: 112 bytes).
#[repr(C)]
pub struct GlyphSlotRec {
    _head: [u8; 112],
    /// Unhinted advance, 16.16 px.
    pub linear_hori_advance: c_long,
    _linear_vert_advance: c_long,
    pub advance: Vector,
    _format: [u8; 8],
    pub bitmap: Bitmap,
    pub bitmap_left: c_int,
    pub bitmap_top: c_int,
}

/// FT_Size_Metrics (26.6 values).
#[repr(C)]
pub struct SizeMetrics {
    pub x_ppem: u16,
    pub y_ppem: u16,
    pub x_scale: c_long,
    pub y_scale: c_long,
    pub ascender: c_long,
    pub descender: c_long,
    pub height: c_long,
    pub max_advance: c_long,
}

/// FT_SizeRec up to metrics.
#[repr(C)]
pub struct SizeRec {
    _head: [u8; 24],
    pub metrics: SizeMetrics,
}

/// FT_FaceRec up to size.
#[repr(C)]
pub struct FaceRec {
    _head: [u8; 152],
    pub glyph: *mut GlyphSlotRec,
    pub size: *mut SizeRec,
}

#[link(name = "freetype")]
unsafe extern "C" {
    fn FT_Init_FreeType(lib: *mut *mut LibraryRec) -> FtError;
    fn FT_Done_FreeType(lib: *mut LibraryRec) -> FtError;
    fn FT_New_Face(
        lib: *mut LibraryRec,
        path: *const c_char,
        index: c_long,
        face: *mut *mut FaceRec,
    ) -> FtError;
    fn FT_Done_Face(face: *mut FaceRec) -> FtError;
    fn FT_Set_Char_Size(
        face: *mut FaceRec,
        w: c_long,
        h: c_long,
        hres: c_uint,
        vres: c_uint,
    ) -> FtError;
    fn FT_Get_Char_Index(face: *mut FaceRec, code: c_ulong) -> c_uint;
    fn FT_Load_Glyph(face: *mut FaceRec, index: c_uint, flags: i32) -> FtError;
    // void FT_Set_Transform(FT_Face face, FT_Matrix* matrix, FT_Vector* delta);
    fn FT_Set_Transform(face: *mut FaceRec, matrix: *const c_void, delta: *const Vector);
}

/// One FreeType library instance. Faces must not outlive it: [`super::Font`]
/// drops its faces first (field order).
pub struct Library(*mut LibraryRec);

impl Library {
    pub fn new() -> Option<Self> {
        let mut lib = std::ptr::null_mut();
        // SAFETY: valid out-pointer; FreeType fills it on success.
        (unsafe { FT_Init_FreeType(&mut lib) } == 0).then_some(Library(lib))
    }

    pub fn open(&self, path: &Path, index: i32) -> Option<Face> {
        let c = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        let mut face = std::ptr::null_mut();
        // SAFETY: library is live; path is NUL-terminated; out-pointer valid.
        let err = unsafe { FT_New_Face(self.0, c.as_ptr(), c_long::from(index), &mut face) };
        (err == 0).then_some(Face(face))
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: created by FT_Init_FreeType; every face was dropped before.
        unsafe { FT_Done_FreeType(self.0) };
    }
}

pub struct Face(*mut FaceRec);

/// A rendered glyph, borrowed from the face's glyph slot until the next load.
pub struct Rendered<'a> {
    pub bitmap: &'a [u8],
    pub width: usize,
    pub rows: usize,
    pub pitch: usize,
    pub left: i32,
    pub top: i32,
}

impl Face {
    /// Size in points at 96 dpi.
    pub fn set_size(&mut self, pt: f32) -> bool {
        let size = (pt * 64.0).round() as c_long;
        // SAFETY: face is live.
        unsafe { FT_Set_Char_Size(self.0, 0, size, 96, 96) == 0 }
    }

    pub fn has(&self, c: char) -> bool {
        // SAFETY: face is live.
        unsafe { FT_Get_Char_Index(self.0, c_ulong::from(c as u32)) != 0 }
    }

    /// Size metrics (26.6): ascender, descender (negative).
    pub fn metrics(&self) -> (i64, i64) {
        // SAFETY: face and its active size are live; fields are plain integers.
        let m = unsafe { &(*(*self.0).size).metrics };
        (m.ascender, m.descender)
    }

    /// Load and rasterize `c` as gray coverage. Advance is in 26.6 (hinted, whole px).
    pub fn render(&mut self, c: char) -> Option<(Rendered<'_>, i64)> {
        self.render_at(c, 0).map(|(r, adv, _)| (r, adv))
    }

    /// [`Face::render`] shifted right by `dx` (26.6, a subpixel phase), plus the
    /// unhinted advance in px for subpixel-positioned text.
    pub fn render_at(&mut self, c: char, dx: i64) -> Option<(Rendered<'_>, i64, f32)> {
        let delta = Vector {
            x: dx as c_long,
            y: 0,
        };
        // SAFETY: face is live; a null matrix means identity; FreeType copies delta.
        unsafe { FT_Set_Transform(self.0, std::ptr::null(), &raw const delta) };
        // SAFETY: face is live.
        let index = unsafe { FT_Get_Char_Index(self.0, c_ulong::from(c as u32)) };
        // SAFETY: face is live; flags are valid FT_LOAD_* bits.
        if unsafe { FT_Load_Glyph(self.0, index, LOAD_RENDER | LOAD_TARGET_LIGHT) } != 0 {
            return None;
        }
        // SAFETY: after a successful load the slot is valid until the next load,
        // which needs `&mut self`, so the borrow below cannot outlive it.
        let slot = unsafe { &*(*self.0).glyph };
        let b = &slot.bitmap;
        if b.pixel_mode != PIXEL_MODE_GRAY || b.pitch < 0 {
            return None; // color/mono bitmaps: not handled in v1
        }
        let len = b.pitch as usize * b.rows as usize;
        let bitmap = if len == 0 || b.buffer.is_null() {
            &[][..]
        } else {
            // SAFETY: FreeType owns `rows * pitch` bytes at `buffer` while the slot lives.
            unsafe { std::slice::from_raw_parts(b.buffer, len) }
        };
        let r = Rendered {
            bitmap,
            width: b.width as usize,
            rows: b.rows as usize,
            pitch: b.pitch as usize,
            left: slot.bitmap_left,
            top: slot.bitmap_top,
        };
        let linear = slot.linear_hori_advance as f32 / 65536.0;
        Some((r, slot.advance.x, linear))
    }
}

impl Drop for Face {
    fn drop(&mut self) {
        // SAFETY: created by FT_New_Face; dropped once.
        unsafe { FT_Done_Face(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    /// Values printed by a C program with offsetof() against FreeType 2.15 headers.
    #[test]
    fn layouts_match_freetype_headers() {
        assert_eq!(offset_of!(FaceRec, glyph), 152);
        assert_eq!(offset_of!(FaceRec, size), 160);
        assert_eq!(offset_of!(SizeRec, metrics), 24);
        assert_eq!(size_of::<SizeMetrics>(), 56);
        assert_eq!(offset_of!(SizeMetrics, ascender), 24);
        assert_eq!(offset_of!(SizeMetrics, height), 40);
        assert_eq!(offset_of!(GlyphSlotRec, linear_hori_advance), 112);
        assert_eq!(offset_of!(GlyphSlotRec, advance), 128);
        assert_eq!(offset_of!(GlyphSlotRec, bitmap), 152);
        assert_eq!(offset_of!(GlyphSlotRec, bitmap_left), 192);
        assert_eq!(offset_of!(GlyphSlotRec, bitmap_top), 196);
        assert_eq!(size_of::<Bitmap>(), 40);
        assert_eq!(offset_of!(Bitmap, buffer), 16);
        assert_eq!(offset_of!(Bitmap, pixel_mode), 26);
    }
}
