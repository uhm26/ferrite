//! The engine: libmpv plays the file (demux, hardware decode, audio, A/V sync,
//! scaling) and draws each video frame into a GL framebuffer we give it. Its two
//! callbacks only write a byte to a pipe, so the main loop sleeps in poll and wakes
//! exactly when mpv has news or a new frame.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::ptr::null_mut;

use crate::gpu;
use crate::sys::mpv::*;
use crate::sys::wake;

#[derive(Debug)]
pub enum Error {
    Create,
    Call(&'static str, String),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Create => write!(f, "mpv_create failed (out of memory?)"),
            Error::Call(what, why) => write!(f, "mpv {what}: {why}"),
            Error::Io(e) => write!(f, "wake pipe: {e}"),
        }
    }
}

impl std::error::Error for Error {}

fn check(what: &'static str, code: c_int) -> Result<(), Error> {
    if code >= 0 {
        return Ok(());
    }
    // SAFETY: mpv returns a static string for every error code.
    let why = unsafe { CStr::from_ptr(mpv_error_string(code)) };
    Err(Error::Call(what, why.to_string_lossy().into_owned()))
}

/// What the controls show, kept current from mpv's property changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub time: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub title: String,
    pub ended: bool,
}

/// Options set before init. config=no: libmpv's default; the owner's mpv.conf is
/// for the mpv app, vid has its own look and keys.
const OPTIONS: &[(&str, &str)] = &[
    ("vo", "libmpv"),
    ("hwdec", "auto-safe"),
    // Load only the VA-API GL interop: the default probes all, CUDA too (noise on AMD).
    // shortcut: AMD/Intel only; add "auto" back if this ever runs on NVIDIA.
    ("gpu-hwdec-interop", "vaapi"),
    ("keep-open", "yes"),
    ("load-scripts", "no"),
    ("osc", "no"),
    ("input-default-bindings", "no"),
    ("terminal", "no"),
    ("audio-client-name", "vid"),
    // Deliver each frame at its display time instead of early + a blocking wait in
    // render (up to 50 ms on our UI thread otherwise; render.h BLOCK_FOR_TARGET_TIME).
    ("video-timing-offset", "0"),
];

/// Observed properties, by reply id.
const WATCH: &[(&str, c_int)] = &[
    ("time-pos", MPV_FORMAT_DOUBLE),
    ("duration", MPV_FORMAT_DOUBLE),
    ("pause", MPV_FORMAT_FLAG),
    ("volume", MPV_FORMAT_DOUBLE),
    ("mute", MPV_FORMAT_FLAG),
    ("media-title", MPV_FORMAT_STRING),
    ("eof-reached", MPV_FORMAT_FLAG),
    ("hwdec-current", MPV_FORMAT_STRING),
];

pub struct Player {
    h: *mut mpv_handle,
    render: *mut mpv_render_context,
    wake_r: File,
    /// Kept open while mpv may call `wake` with its number.
    _wake_w: OwnedFd,
    pub state: State,
    /// mpv shut down (window closed elsewhere, `quit` command).
    pub quit: bool,
    /// The file failed to load or play.
    pub failed: Option<String>,
}

impl Player {
    /// Start mpv on `file` drawing through our GL context (`gpu` must be current).
    pub fn new(file: &str, gpu: &gpu::Gpu) -> Result<Player, Error> {
        let (r, w) = frame::sys::pipe().map_err(Error::Io)?;
        frame::sys::set_nonblocking(r.as_fd()).map_err(Error::Io)?;
        frame::sys::set_nonblocking(w.as_fd()).map_err(Error::Io)?;
        // SAFETY: no arguments; null on failure.
        let h = unsafe { mpv_create() };
        if h.is_null() {
            return Err(Error::Create);
        }
        let mut p = Player {
            h,
            render: null_mut(),
            wake_r: File::from(r),
            _wake_w: w,
            state: State::default(),
            quit: false,
            failed: None,
        };
        for (k, v) in OPTIONS {
            p.option(k, v)?;
        }
        // SAFETY: live handle, options set.
        check("initialize", unsafe { mpv_initialize(p.h) })?;
        p.render = render_context(p.h, gpu.node_fd())?;
        let d = p._wake_w.as_raw_fd() as usize as *mut c_void;
        // SAFETY: `wake` only writes to the pipe, which outlives both callbacks
        // (Drop frees the render context and mpv first).
        unsafe {
            mpv_render_context_set_update_callback(p.render, Some(wake), d);
            mpv_set_wakeup_callback(p.h, Some(wake), d);
        }
        for (i, (name, format)) in WATCH.iter().enumerate() {
            let name = CString::new(*name).expect("no NUL in literals");
            // SAFETY: live handle, NUL-terminated name, a format mpv knows.
            check("observe", unsafe {
                mpv_observe_property(p.h, i as u64, name.as_ptr(), *format)
            })?;
        }
        p.command(&["loadfile", file])?;
        Ok(p)
    }

    fn option(&mut self, k: &str, v: &str) -> Result<(), Error> {
        let (k, v) = (
            CString::new(k).expect("literal"),
            CString::new(v).expect("literal"),
        );
        // SAFETY: live handle, both strings NUL-terminated for the call.
        check("set_option", unsafe {
            mpv_set_option_string(self.h, k.as_ptr(), v.as_ptr())
        })
    }

    /// Run an mpv input command, e.g. `["seek", "5", "relative"]`.
    pub fn command(&mut self, args: &[&str]) -> Result<(), Error> {
        let owned: Vec<CString> = args
            .iter()
            .map(|a| CString::new(*a).map_err(|_| Error::Call("command", "NUL in argument".into())))
            .collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: null-terminated array of strings that live through the call.
        check("command", unsafe { mpv_command(self.h, ptrs.as_mut_ptr()) })
    }

    pub fn set_flag(&mut self, name: &str, on: bool) -> Result<(), Error> {
        let mut v: c_int = on.into();
        let name = CString::new(name).expect("literal");
        // SAFETY: live handle; FLAG reads one int from `v`.
        check("set_property", unsafe {
            mpv_set_property(self.h, name.as_ptr(), MPV_FORMAT_FLAG, (&raw mut v).cast())
        })
    }

    pub fn set_double(&mut self, name: &str, v: f64) -> Result<(), Error> {
        let mut v = v;
        let name = CString::new(name).expect("literal");
        // SAFETY: live handle; DOUBLE reads one f64 from `v`.
        check("set_property", unsafe {
            mpv_set_property(
                self.h,
                name.as_ptr(),
                MPV_FORMAT_DOUBLE,
                (&raw mut v).cast(),
            )
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake_r.as_fd()
    }

    /// Empty the wake pipe and apply every queued mpv event. True if the state the
    /// controls show changed.
    pub fn poll(&mut self) -> bool {
        let mut sink = [0u8; 64];
        while matches!((&self.wake_r).read(&mut sink), Ok(n) if n > 0) {}
        let before = self.state.clone();
        loop {
            // SAFETY: live handle; timeout 0 never blocks. The event stays valid
            // until the next wait_event call.
            let ev = unsafe { &*mpv_wait_event(self.h, 0.0) };
            match ev.event_id {
                MPV_EVENT_NONE => break,
                MPV_EVENT_SHUTDOWN => self.quit = true,
                MPV_EVENT_PROPERTY_CHANGE => self.property(ev),
                MPV_EVENT_END_FILE => self.end_file(ev),
                _ => {}
            }
        }
        self.state != before
    }

    fn property(&mut self, ev: &mpv_event) {
        // SAFETY: PROPERTY_CHANGE carries an mpv_event_property.
        let p = unsafe { &*(ev.data as *const mpv_event_property) };
        let Some((_, format)) = WATCH.get(ev.reply_userdata as usize) else {
            return;
        };
        if p.data.is_null() || p.format != *format {
            return; // unavailable for now (no file yet): keep the last value
        }
        let s = &mut self.state;
        // SAFETY: the data pointer has the type of the format we asked for.
        unsafe {
            match ev.reply_userdata {
                0 => s.time = *(p.data as *const f64),
                1 => s.duration = *(p.data as *const f64),
                2 => s.paused = *(p.data as *const c_int) != 0,
                3 => s.volume = *(p.data as *const f64),
                4 => s.muted = *(p.data as *const c_int) != 0,
                5 => {
                    let c = *(p.data as *const *const c_char);
                    s.title = CStr::from_ptr(c).to_string_lossy().into_owned();
                }
                6 => s.ended = *(p.data as *const c_int) != 0,
                7 => {
                    let c = *(p.data as *const *const c_char);
                    eprintln!("vid: decoder: {}", CStr::from_ptr(c).to_string_lossy());
                }
                _ => {}
            }
        }
    }

    fn end_file(&mut self, ev: &mpv_event) {
        // SAFETY: END_FILE carries an mpv_event_end_file.
        let e = unsafe { &*(ev.data as *const mpv_event_end_file) };
        if e.reason == MPV_END_FILE_REASON_ERROR {
            // SAFETY: mpv returns a static string for every error code.
            let why = unsafe { CStr::from_ptr(mpv_error_string(e.error)) };
            let why = why.to_string_lossy().into_owned();
            eprintln!("vid: can't play: {why}");
            self.failed = Some(why);
        }
    }

    /// mpv has a new frame for us to draw.
    pub fn frame_due(&mut self) -> bool {
        // SAFETY: live render context, on the thread with the GL context.
        unsafe { mpv_render_context_update(self.render) & MPV_RENDER_UPDATE_FRAME != 0 }
    }

    /// Draw the current frame into `t`'s framebuffer.
    pub fn render(&mut self, t: &gpu::Target) -> Result<(), Error> {
        let mut fbo = mpv_opengl_fbo {
            fbo: t.fbo as c_int,
            w: t.w as c_int,
            h: t.h as c_int,
            internal_format: 0,
        };
        // Our buffer is read top row first, like an image: no flip.
        let mut flip: c_int = 0;
        let mut params = [
            param(MPV_RENDER_PARAM_OPENGL_FBO, (&raw mut fbo).cast()),
            param(MPV_RENDER_PARAM_FLIP_Y, (&raw mut flip).cast()),
            param(MPV_RENDER_PARAM_INVALID, null_mut()),
        ];
        // SAFETY: params end in INVALID and point at locals that outlive the call;
        // the GL context is current.
        check("render", unsafe {
            mpv_render_context_render(self.render, params.as_mut_ptr())
        })
    }

    /// The frame we drew reached the screen (timing statistics for mpv).
    pub fn report_swap(&mut self) {
        // SAFETY: live render context.
        unsafe { mpv_render_context_report_swap(self.render) };
    }
}

fn param(kind: c_int, data: *mut c_void) -> mpv_render_param {
    mpv_render_param { kind, data }
}

/// The GL render context, sharing the render node with mpv's VA-API decoder.
fn render_context(h: *mut mpv_handle, node: c_int) -> Result<*mut mpv_render_context, Error> {
    let mut init = mpv_opengl_init_params {
        get_proc_address: Some(gpu::proc_address),
        get_proc_address_ctx: null_mut(),
    };
    let mut drm = mpv_opengl_drm_params_v2 {
        fd: -1,
        crtc_id: -1,
        connector_id: -1,
        atomic_request_ptr: null_mut(),
        render_fd: node,
    };
    let mut params = [
        param(
            MPV_RENDER_PARAM_API_TYPE,
            c"opengl".as_ptr().cast_mut().cast(),
        ),
        param(MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, (&raw mut init).cast()),
        param(MPV_RENDER_PARAM_DRM_DISPLAY_V2, (&raw mut drm).cast()),
        param(MPV_RENDER_PARAM_INVALID, null_mut()),
    ];
    let mut ctx = null_mut();
    // SAFETY: params end in INVALID, point at locals that live through the call
    // (mpv copies them), and the GL context is current.
    check("render_context_create", unsafe {
        mpv_render_context_create(&mut ctx, h, params.as_mut_ptr())
    })?;
    Ok(ctx)
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: render context first (render.h: before the mpv handle), both ours;
        // the GL context is still current (Gpu drops after us).
        unsafe {
            if !self.render.is_null() {
                mpv_render_context_free(self.render);
            }
            mpv_terminate_destroy(self.h);
        }
    }
}
