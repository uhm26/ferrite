//! The GPU side: a GL context with no window (EGL on GBM, the render node), and
//! render targets that are GPU buffers mpv draws into and the compositor shows
//! as they are (dmabufs: no copy, no CPU).

use std::ffi::{c_char, c_int, c_void};
use std::fmt;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr::null_mut;

use crate::sys::egl::{self, EGLDisplay, EGLImage, Gl};
use crate::sys::gbm;

/// shortcut: first render node, fine on a one-GPU box; pick the compositor's
/// (dmabuf feedback main_device) once a second GPU shows up.
const NODE: &str = "/dev/dri/renderD128";
/// drm_fourcc.h: DRM_FORMAT_MOD_INVALID, "no explicit modifier".
pub const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

#[derive(Debug)]
pub enum Error {
    Node(std::io::Error),
    Gbm(&'static str),
    Egl(&'static str, i32),
    GlMissing,
    Framebuffer(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Node(e) => write!(f, "open {NODE}: {e}"),
            Error::Gbm(what) => write!(f, "gbm: {what} failed"),
            Error::Egl(what, code) => write!(f, "egl: {what} failed (0x{code:x})"),
            Error::GlMissing => write!(f, "the GL driver lacks a function we need"),
            Error::Framebuffer(s) => write!(f, "GL framebuffer incomplete (0x{s:x})"),
        }
    }
}

impl std::error::Error for Error {}

fn egl_err(what: &'static str) -> Error {
    // SAFETY: no arguments; reads this thread's last EGL error.
    Error::Egl(what, unsafe { egl::eglGetError() })
}

pub struct Gpu {
    node: File,
    dev: *mut gbm::gbm_device,
    dpy: EGLDisplay,
    ctx: egl::EGLContext,
    pub gl: Gl,
}

impl Gpu {
    /// Open the render node and make a GL context current on this thread.
    pub fn open() -> Result<Gpu, Error> {
        let node = File::options()
            .read(true)
            .write(true)
            .open(NODE)
            .map_err(Error::Node)?;
        // SAFETY: a valid open DRM fd; gbm keeps no ownership of it.
        let dev = unsafe { gbm::gbm_create_device(node.as_raw_fd()) };
        if dev.is_null() {
            return Err(Error::Gbm("gbm_create_device"));
        }
        // SAFETY: GBM platform with our live gbm device; no attributes.
        let dpy = unsafe {
            egl::eglGetPlatformDisplay(egl::EGL_PLATFORM_GBM_KHR, dev.cast(), null_mut())
        };
        // SAFETY: `dpy` from above; null version out-pointers are allowed.
        if dpy.is_null() || unsafe { egl::eglInitialize(dpy, null_mut(), null_mut()) } == 0 {
            // SAFETY: we made `dev` above and nothing else uses it.
            unsafe { gbm::gbm_device_destroy(dev) };
            return Err(egl_err("eglInitialize"));
        }
        let ctx = context(dpy)?;
        let gl = Gl::load().ok_or(Error::GlMissing)?;
        Ok(Gpu {
            node,
            dev,
            dpy,
            ctx,
            gl,
        })
    }

    /// The render node, for mpv's hardware decoder (same GPU, zero-copy frames).
    pub fn node_fd(&self) -> RawFd {
        self.node.as_raw_fd()
    }

    /// Send the queued GL work to the GPU without waiting for it.
    /// shortcut: relies on amdgpu implicit sync (the compositor's import waits on the
    /// dmabuf's fence), not checked live yet; if Hyprland ever shows half-drawn
    /// frames, use explicit sync (linux-drm-syncobj) — glFinish was the old stopgap.
    pub fn flush(&self) {
        // SAFETY: our context is current on this thread.
        unsafe { (self.gl.flush)() };
    }

    /// A `w` × `h` XRGB buffer mpv can draw into (`fbo`) and Wayland can show, laid
    /// out with one of `modifiers` (the compositor's list; empty: gbm picks).
    pub fn target(&self, w: u32, h: u32, modifiers: &[u64]) -> Result<Target, Error> {
        let bo = self.bo(w, h, modifiers);
        if bo.is_null() {
            return Err(Error::Gbm("gbm_bo_create"));
        }
        let mut t = Target {
            bo,
            image: null_mut(),
            tex: 0,
            fbo: 0,
            w,
            h,
            planes: Vec::new(),
            // SAFETY: live bo.
            modifier: unsafe { gbm::gbm_bo_get_modifier(bo) },
            dpy: self.dpy,
            delete_tex: self.gl.delete_textures,
            delete_fbo: self.gl.delete_framebuffers,
        };
        t.export()?;
        t.image = self.import(&t)?;
        self.attach(&mut t)?;
        Ok(t)
    }

    /// A gbm buffer with a modifier from `modifiers`, else one gbm picks (also if
    /// gbm can't use any of them).
    fn bo(&self, w: u32, h: u32, modifiers: &[u64]) -> *mut gbm::gbm_bo {
        let (fmt, usage) = (gbm::GBM_FORMAT_XRGB8888, gbm::GBM_BO_USE_RENDERING);
        if !modifiers.is_empty() {
            let (p, n) = (modifiers.as_ptr(), modifiers.len() as u32);
            // SAFETY: live device; `p` points at `n` modifiers that outlive the call.
            let bo =
                unsafe { gbm::gbm_bo_create_with_modifiers2(self.dev, w, h, fmt, p, n, usage) };
            if !bo.is_null() {
                return bo;
            }
        }
        // SAFETY: live device; XRGB8888 for rendering is supported by every mesa driver.
        unsafe { gbm::gbm_bo_create(self.dev, w, h, fmt, usage) }
    }

    fn import(&self, t: &Target) -> Result<EGLImage, Error> {
        let mut a: Vec<egl::EGLAttrib> = vec![
            egl::EGL_WIDTH,
            t.w as isize,
            egl::EGL_HEIGHT,
            t.h as isize,
            egl::EGL_LINUX_DRM_FOURCC_EXT,
            gbm::GBM_FORMAT_XRGB8888 as isize,
        ];
        for (n, p) in t.planes.iter().enumerate() {
            let [fd, off, pitch] = egl::plane_attrs(n);
            a.extend([
                fd,
                p.fd.as_raw_fd() as isize,
                off,
                p.offset as isize,
                pitch,
                p.stride as isize,
            ]);
            if t.modifier != MOD_INVALID {
                let [lo, hi] = egl::modifier_attrs(n);
                a.extend([
                    lo,
                    (t.modifier & 0xffff_ffff) as isize,
                    hi,
                    (t.modifier >> 32) as isize,
                ]);
            }
        }
        a.push(egl::EGL_NONE as isize);
        // SAFETY: attribute list ends in EGL_NONE; the fds stay open in `t.planes`.
        let image = unsafe {
            egl::eglCreateImage(
                self.dpy,
                null_mut(),
                egl::EGL_LINUX_DMA_BUF_EXT,
                null_mut(),
                a.as_ptr(),
            )
        };
        if image.is_null() {
            return Err(egl_err("eglCreateImage"));
        }
        Ok(image)
    }

    /// Back a texture with the image and a framebuffer with the texture.
    fn attach(&self, t: &mut Target) -> Result<(), Error> {
        let gl = &self.gl;
        // SAFETY: our context is current; every pointer is to a live local or field.
        unsafe {
            (gl.gen_textures)(1, &mut t.tex);
            (gl.bind_texture)(egl::GL_TEXTURE_2D, t.tex);
            (gl.image_target_texture)(egl::GL_TEXTURE_2D, t.image);
            (gl.bind_texture)(egl::GL_TEXTURE_2D, 0);
            (gl.gen_framebuffers)(1, &mut t.fbo);
            (gl.bind_framebuffer)(egl::GL_FRAMEBUFFER, t.fbo);
            let (fb, color, tex2d) = (
                egl::GL_FRAMEBUFFER,
                egl::GL_COLOR_ATTACHMENT0,
                egl::GL_TEXTURE_2D,
            );
            (gl.framebuffer_texture_2d)(fb, color, tex2d, t.tex, 0);
            let status = (gl.check_framebuffer_status)(fb);
            (gl.bind_framebuffer)(fb, 0);
            if status != egl::GL_FRAMEBUFFER_COMPLETE {
                return Err(Error::Framebuffer(status));
            }
        }
        Ok(())
    }
}

/// mpv's render API looks GL functions up through this.
///
/// # Safety
/// `name` is a NUL-terminated string (mpv passes literals).
pub unsafe extern "C" fn proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: forwarded as is; EGL returns null for unknown names.
    unsafe { egl::eglGetProcAddress(name) }
}

/// Make a desktop GL core context (4.6, else 3.3) current, without a surface.
fn context(dpy: EGLDisplay) -> Result<egl::EGLContext, Error> {
    // SAFETY: no pointers.
    if unsafe { egl::eglBindAPI(egl::EGL_OPENGL_API) } == 0 {
        return Err(egl_err("eglBindAPI"));
    }
    for (major, minor) in [(4, 6), (3, 3)] {
        let attrs = [
            egl::EGL_CONTEXT_MAJOR_VERSION,
            major,
            egl::EGL_CONTEXT_MINOR_VERSION,
            minor,
            egl::EGL_CONTEXT_OPENGL_PROFILE_MASK,
            egl::EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::EGL_NONE,
        ];
        // SAFETY: no config (EGL_KHR_no_config_context) and no share context; the
        // list ends in EGL_NONE.
        let ctx = unsafe { egl::eglCreateContext(dpy, null_mut(), null_mut(), attrs.as_ptr()) };
        if ctx.is_null() {
            continue;
        }
        // SAFETY: surfaceless (EGL_KHR_surfaceless_context): mpv draws into our FBOs.
        if unsafe { egl::eglMakeCurrent(dpy, null_mut(), null_mut(), ctx) } == 0 {
            return Err(egl_err("eglMakeCurrent"));
        }
        return Ok(ctx);
    }
    Err(egl_err("eglCreateContext"))
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: we own all three; targets (which borrow dpy) are dropped first by
        // the owner (field order in app.rs).
        unsafe {
            egl::eglMakeCurrent(self.dpy, null_mut(), null_mut(), null_mut());
            egl::eglDestroyContext(self.dpy, self.ctx);
            egl::eglTerminate(self.dpy);
            gbm::gbm_device_destroy(self.dev);
        }
    }
}

/// One dmabuf plane: the fd, where it starts, its row pitch.
pub struct Plane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

/// A GPU buffer with a GL framebuffer over it.
pub struct Target {
    bo: *mut gbm::gbm_bo,
    image: EGLImage,
    tex: u32,
    pub fbo: u32,
    pub w: u32,
    pub h: u32,
    pub planes: Vec<Plane>,
    pub modifier: u64,
    dpy: EGLDisplay,
    delete_tex: unsafe extern "C" fn(c_int, *const u32),
    delete_fbo: unsafe extern "C" fn(c_int, *const u32),
}

impl Target {
    fn export(&mut self) -> Result<(), Error> {
        // SAFETY: live bo.
        let n = unsafe { gbm::gbm_bo_get_plane_count(self.bo) };
        for p in 0..n {
            // SAFETY: live bo, plane < count; the fd is new and ours.
            let (fd, offset, stride) = unsafe {
                let fd = gbm::gbm_bo_get_fd_for_plane(self.bo, p);
                (
                    fd,
                    gbm::gbm_bo_get_offset(self.bo, p),
                    gbm::gbm_bo_get_stride_for_plane(self.bo, p),
                )
            };
            if fd < 0 {
                return Err(Error::Gbm("gbm_bo_get_fd_for_plane"));
            }
            // SAFETY: gbm just returned this fd for us to own.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            self.planes.push(Plane { fd, offset, stride });
        }
        Ok(())
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        // SAFETY: we own fbo, texture, image and bo; the GL context is current on
        // this (the only) thread; zero names and null images are skipped.
        unsafe {
            if self.fbo != 0 {
                (self.delete_fbo)(1, &self.fbo);
            }
            if self.tex != 0 {
                (self.delete_tex)(1, &self.tex);
            }
            if !self.image.is_null() {
                egl::eglDestroyImage(self.dpy, self.image);
            }
            gbm::gbm_bo_destroy(self.bo);
        }
    }
}
