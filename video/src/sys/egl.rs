//! Raw EGL 1.5 (libEGL) and the few GL calls we make (loaded through
//! eglGetProcAddress, so no libGL link). Signatures from /usr/include/EGL/egl.h,
//! EGL/eglext.h and GL/glcorearb.h.

use std::ffi::{c_char, c_int, c_uint, c_void};

pub type EGLDisplay = *mut c_void;
pub type EGLContext = *mut c_void;
pub type EGLImage = *mut c_void;
pub type EGLAttrib = isize;
pub type EGLint = i32;

pub const EGL_NONE: EGLint = 0x3038;
pub const EGL_OPENGL_API: c_uint = 0x30A2;
pub const EGL_CONTEXT_MAJOR_VERSION: EGLint = 0x3098;
pub const EGL_CONTEXT_MINOR_VERSION: EGLint = 0x30FB;
pub const EGL_CONTEXT_OPENGL_PROFILE_MASK: EGLint = 0x30FD;
pub const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT: EGLint = 1;
pub const EGL_PLATFORM_GBM_KHR: c_uint = 0x31D7;
pub const EGL_WIDTH: EGLAttrib = 0x3057;
pub const EGL_HEIGHT: EGLAttrib = 0x3056;
pub const EGL_LINUX_DMA_BUF_EXT: c_uint = 0x3270;
pub const EGL_LINUX_DRM_FOURCC_EXT: EGLAttrib = 0x3271;

/// EGL_DMA_BUF_PLANE{n}_{FD,OFFSET,PITCH}_EXT (eglext.h: planes 0-2 are 0x3272 + 3n,
/// plane 3 is 0x3440).
pub const fn plane_attrs(n: usize) -> [EGLAttrib; 3] {
    let base = if n < 3 {
        0x3272 + 3 * n as EGLAttrib
    } else {
        0x3440
    };
    [base, base + 1, base + 2]
}

/// EGL_DMA_BUF_PLANE{n}_MODIFIER_{LO,HI}_EXT.
pub const fn modifier_attrs(n: usize) -> [EGLAttrib; 2] {
    let lo = 0x3443 + 2 * n as EGLAttrib;
    [lo, lo + 1]
}

#[link(name = "EGL")]
unsafe extern "C" {
    pub fn eglGetPlatformDisplay(
        platform: c_uint,
        native: *mut c_void,
        attribs: *const EGLAttrib,
    ) -> EGLDisplay;
    pub fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> c_uint;
    pub fn eglTerminate(dpy: EGLDisplay) -> c_uint;
    pub fn eglBindAPI(api: c_uint) -> c_uint;
    pub fn eglCreateContext(
        dpy: EGLDisplay,
        config: *mut c_void,
        share: EGLContext,
        attribs: *const EGLint,
    ) -> EGLContext;
    pub fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> c_uint;
    pub fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: *mut c_void,
        read: *mut c_void,
        ctx: EGLContext,
    ) -> c_uint;
    pub fn eglCreateImage(
        dpy: EGLDisplay,
        ctx: EGLContext,
        target: c_uint,
        buffer: *mut c_void,
        attribs: *const EGLAttrib,
    ) -> EGLImage;
    pub fn eglDestroyImage(dpy: EGLDisplay, image: EGLImage) -> c_uint;
    pub fn eglGetError() -> EGLint;
    pub fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
}

pub const GL_TEXTURE_2D: c_uint = 0x0DE1;
pub const GL_FRAMEBUFFER: c_uint = 0x8D40;
pub const GL_COLOR_ATTACHMENT0: c_uint = 0x8CE0;
pub const GL_FRAMEBUFFER_COMPLETE: c_uint = 0x8CD5;

/// The GL entry points we use, resolved once with the context current.
pub struct Gl {
    pub gen_textures: unsafe extern "C" fn(c_int, *mut c_uint),
    pub delete_textures: unsafe extern "C" fn(c_int, *const c_uint),
    pub bind_texture: unsafe extern "C" fn(c_uint, c_uint),
    /// glEGLImageTargetTexture2DOES (GL_OES_EGL_image)
    pub image_target_texture: unsafe extern "C" fn(c_uint, EGLImage),
    pub gen_framebuffers: unsafe extern "C" fn(c_int, *mut c_uint),
    pub delete_framebuffers: unsafe extern "C" fn(c_int, *const c_uint),
    pub bind_framebuffer: unsafe extern "C" fn(c_uint, c_uint),
    pub framebuffer_texture_2d: unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint, c_int),
    pub check_framebuffer_status: unsafe extern "C" fn(c_uint) -> c_uint,
    pub finish: unsafe extern "C" fn(),
}

/// Look up `name` (NUL-terminated) as a function pointer of type `F`.
///
/// # Safety
/// `F` must be the C signature of `name`, and a GL context must be current.
unsafe fn load<F: Copy>(name: &[u8]) -> Option<F> {
    // SAFETY: `name` ends in NUL (checked by every caller's literal).
    let p = unsafe { eglGetProcAddress(name.as_ptr().cast()) };
    // SAFETY: a non-null result is the function, whose type the caller vouches for;
    // F is a fn pointer, the same size as a data pointer on this platform.
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

impl Gl {
    /// None if the driver lacks one of them.
    pub fn load() -> Option<Gl> {
        // SAFETY: every name below is paired with its signature from glcorearb.h /
        // gl2ext.h; the caller made a context current.
        unsafe {
            Some(Gl {
                gen_textures: load(b"glGenTextures\0")?,
                delete_textures: load(b"glDeleteTextures\0")?,
                bind_texture: load(b"glBindTexture\0")?,
                image_target_texture: load(b"glEGLImageTargetTexture2DOES\0")?,
                gen_framebuffers: load(b"glGenFramebuffers\0")?,
                delete_framebuffers: load(b"glDeleteFramebuffers\0")?,
                bind_framebuffer: load(b"glBindFramebuffer\0")?,
                framebuffer_texture_2d: load(b"glFramebufferTexture2D\0")?,
                check_framebuffer_status: load(b"glCheckFramebufferStatus\0")?,
                finish: load(b"glFinish\0")?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmabuf_attribute_numbers_match_eglext() {
        assert_eq!(plane_attrs(0), [0x3272, 0x3273, 0x3274]);
        assert_eq!(plane_attrs(2), [0x3278, 0x3279, 0x327A]);
        assert_eq!(plane_attrs(3), [0x3440, 0x3441, 0x3442]);
        assert_eq!(modifier_attrs(0), [0x3443, 0x3444]);
        assert_eq!(modifier_attrs(3), [0x3449, 0x344A]);
    }
}
