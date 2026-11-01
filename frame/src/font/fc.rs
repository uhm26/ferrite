//! Own fontconfig bindings: find a font file by name, or one that has a given
//! character (fallback). Signatures from /usr/include/fontconfig/fontconfig.h (2.18).
//! Copied into frame from ../term/src/font/fc.rs at term 7be66f5 (2026-11-01).

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

type FcPattern = c_void;
type FcCharSet = c_void;

const FC_MATCH_PATTERN: c_int = 0; // FcMatchPattern
const FC_RESULT_MATCH: c_int = 0; // FcResultMatch

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcNameParse(name: *const u8) -> *mut FcPattern;
    fn FcConfigSubstitute(config: *mut c_void, p: *mut FcPattern, kind: c_int) -> c_int;
    fn FcDefaultSubstitute(p: *mut FcPattern);
    fn FcFontMatch(config: *mut c_void, p: *mut FcPattern, result: *mut c_int) -> *mut FcPattern;
    fn FcPatternGetString(
        p: *const FcPattern,
        obj: *const c_char,
        n: c_int,
        s: *mut *mut u8,
    ) -> c_int;
    fn FcPatternGetInteger(
        p: *const FcPattern,
        obj: *const c_char,
        n: c_int,
        i: *mut c_int,
    ) -> c_int;
    fn FcPatternAddCharSet(p: *mut FcPattern, obj: *const c_char, c: *const FcCharSet) -> c_int;
    fn FcPatternDestroy(p: *mut FcPattern);
    fn FcCharSetCreate() -> *mut FcCharSet;
    fn FcCharSetAddChar(c: *mut FcCharSet, ucs4: u32) -> c_int;
    fn FcCharSetDestroy(c: *mut FcCharSet);
}

/// An owned FcPattern.
struct Pattern(*mut FcPattern);

impl Drop for Pattern {
    fn drop(&mut self) {
        // SAFETY: we own this pattern and destroy it once.
        unsafe { FcPatternDestroy(self.0) };
    }
}

/// Best font file (path, face index) for a fontconfig name like `Adwaita Mono:bold`,
/// optionally one that contains `needs`.
pub fn find(name: &str, needs: Option<char>) -> Option<(PathBuf, i32)> {
    let name = CString::new(name).ok()?;
    // SAFETY: NUL-terminated name; a null result is checked.
    let p = unsafe { FcNameParse(name.as_ptr().cast()) };
    if p.is_null() {
        return None;
    }
    let p = Pattern(p);
    if let Some(c) = needs {
        // SAFETY: a fresh charset; the pattern copies it, so we destroy ours after.
        unsafe {
            let cs = FcCharSetCreate();
            FcCharSetAddChar(cs, c as u32);
            FcPatternAddCharSet(p.0, c"charset".as_ptr(), cs);
            FcCharSetDestroy(cs);
        }
    }
    let mut result = 0;
    // SAFETY: pattern is live; null config means the default config.
    let m = unsafe {
        FcConfigSubstitute(std::ptr::null_mut(), p.0, FC_MATCH_PATTERN);
        FcDefaultSubstitute(p.0);
        FcFontMatch(std::ptr::null_mut(), p.0, &mut result)
    };
    if m.is_null() {
        return None;
    }
    let m = Pattern(m);
    file_of(&m)
}

fn file_of(m: &Pattern) -> Option<(PathBuf, i32)> {
    let mut file: *mut u8 = std::ptr::null_mut();
    let mut index: c_int = 0;
    // SAFETY: `m` is live; out-pointers valid; the string belongs to `m`, so we
    // copy it before `m` is dropped.
    unsafe {
        if FcPatternGetString(m.0, c"file".as_ptr(), 0, &mut file) != FC_RESULT_MATCH {
            return None;
        }
        FcPatternGetInteger(m.0, c"index".as_ptr(), 0, &mut index);
        let bytes = CStr::from_ptr(file.cast()).to_bytes();
        Some((PathBuf::from(std::ffi::OsStr::from_bytes(bytes)), index))
    }
}
