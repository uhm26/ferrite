//! The video layer: a subsurface under the window (frame's controls are drawn on
//! the window above it, see-through where there are none), showing mpv's frames as
//! dmabufs. Desynced from the window, so a video frame needs no window redraw.
//! Up to three buffers: one on screen, one queued, one being drawn.

use frame::window::Window;
use frame::wl::{Arg, Error, Event as WlEvent};

use crate::gpu::{Gpu, MOD_INVALID, Target};
use crate::sys::gbm::GBM_FORMAT_XRGB8888;

const SLOTS: usize = 3;

// wayland.xml / linux-dmabuf-v1.xml request and event numbers (XML order).
const GET_SUBSURFACE: u16 = 1;
const PLACE_BELOW: u16 = 3;
const SET_DESYNC: u16 = 5;
const CREATE_REGION: u16 = 1;
const REGION_DESTROY: u16 = 0;
const SET_INPUT_REGION: u16 = 5;
const SURFACE_ATTACH: u16 = 1;
const SURFACE_FRAME: u16 = 3;
const SURFACE_COMMIT: u16 = 6;
const SURFACE_DAMAGE_BUFFER: u16 = 9;
const CREATE_PARAMS: u16 = 1;
const PARAMS_DESTROY: u16 = 0;
const PARAMS_ADD: u16 = 1;
const PARAMS_CREATE_IMMED: u16 = 3;
const BUFFER_DESTROY: u16 = 0;
/// wl_buffer.release and wl_callback.done (the same number; told apart by object).
const EV_RELEASE_OR_DONE: u16 = 0;
/// zwp_linux_dmabuf_v1.modifier: format, modifier_hi, modifier_lo.
const EV_DMABUF_MODIFIER: u16 = 1;

struct Slot {
    target: Target,
    buffer: u32,
    busy: bool,
}

pub struct Video {
    surface: u32,
    dmabuf: u32,
    /// Modifiers the compositor imports for XRGB8888 (its v3 events after bind).
    modifiers: Vec<u64>,
    /// Buffers of any size: one of an old size goes when it is free (see `acquire`).
    slots: Vec<Slot>,
    /// One frame callback per presented frame, so mpv gets one swap report each.
    frame_cbs: Vec<u32>,
    size: (u32, u32),
    /// A frame is on screen (until then the window paints black itself).
    pub shown: bool,
}

impl Video {
    pub fn new<S>(win: &mut Window<S>) -> Result<Video, Error> {
        let (sub, _) = win
            .bind_global("wl_subcompositor", 1)?
            .ok_or(Error::Missing("wl_subcompositor"))?;
        let (dmabuf, _) = win
            .bind_global("zwp_linux_dmabuf_v1", 3)?
            .ok_or(Error::Missing("zwp_linux_dmabuf_v1 v3"))?;
        win.claim(dmabuf); // its modifier events, for `on_event`
        let (parent, comp) = (win.surface_id(), win.compositor_id());
        let c = win.conn();
        let surface = c.create_surface(comp)?;
        let subsurface = c.new_id();
        c.send(
            sub,
            GET_SUBSURFACE,
            &[Arg::U(subsurface), Arg::U(surface), Arg::U(parent)],
        )?;
        c.send(subsurface, PLACE_BELOW, &[Arg::U(parent)])?;
        c.send(subsurface, SET_DESYNC, &[])?;
        c.opaque_everywhere(comp, surface)?;
        let empty = c.new_id(); // the pointer goes to the window, never to us
        c.send(comp, CREATE_REGION, &[Arg::U(empty)])?;
        c.send(surface, SET_INPUT_REGION, &[Arg::U(empty)])?;
        c.send(empty, REGION_DESTROY, &[])?;
        c.send(surface, SURFACE_COMMIT, &[])?;
        Ok(Video {
            surface,
            dmabuf,
            modifiers: Vec::new(),
            slots: Vec::new(),
            frame_cbs: Vec::new(),
            size: (0, 0),
            shown: false,
        })
    }

    /// Draw at `w` × `h` from the next frame on (the window's size: mpv scales and
    /// letterboxes itself, with its own good filters). Buffers follow lazily in
    /// `acquire`, so a resize drag's many configures cost nothing by themselves.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = (w, h);
    }

    /// A free buffer of the current size to draw into, made if needed. None while
    /// all are on screen, or a new one would have to be made before the compositor
    /// showed the last frame.
    pub fn acquire<S>(
        &mut self,
        win: &mut Window<S>,
        gpu: &Gpu,
    ) -> Result<Option<usize>, Box<dyn std::error::Error>> {
        let (w, h) = self.size;
        let stale = |s: &mut Slot| !s.busy && (s.target.w, s.target.h) != (w, h);
        for s in self.slots.extract_if(.., stale) {
            win.conn().send(s.buffer, BUFFER_DESTROY, &[])?;
        }
        if let Some(i) = self.slots.iter().position(|s| !s.busy) {
            return Ok(Some(i));
        }
        // At most one new buffer per compositor frame: a resize drag reallocates at
        // the screen's rate, not at the configure rate.
        let waiting = !self.frame_cbs.is_empty();
        if self.slots.len() >= SLOTS || w == 0 || h == 0 || waiting {
            return Ok(None);
        }
        let target = gpu.target(w, h, &self.modifiers)?;
        let buffer = self.wl_buffer(win, &target)?;
        self.slots.push(Slot {
            target,
            buffer,
            busy: false,
        });
        Ok(Some(self.slots.len() - 1))
    }

    pub fn target(&self, i: usize) -> &Target {
        &self.slots[i].target
    }

    /// Hand the GPU buffer to the compositor: a wl_buffer over the same memory.
    fn wl_buffer<S>(&mut self, win: &mut Window<S>, t: &Target) -> Result<u32, Error> {
        let c = win.conn();
        let params = c.new_id();
        c.send(self.dmabuf, CREATE_PARAMS, &[Arg::U(params)])?;
        let (hi, lo) = ((t.modifier >> 32) as u32, t.modifier as u32);
        for (n, p) in t.planes.iter().enumerate() {
            use std::os::fd::AsFd;
            let args = [
                Arg::Fd(p.fd.as_fd()),
                Arg::U(n as u32),
                Arg::U(p.offset),
                Arg::U(p.stride),
                Arg::U(hi),
                Arg::U(lo),
            ];
            c.send(params, PARAMS_ADD, &args)?;
        }
        let buffer = c.new_id();
        let (w, h) = (t.w as i32, t.h as i32);
        let args = [
            Arg::U(buffer),
            Arg::I(w),
            Arg::I(h),
            Arg::U(GBM_FORMAT_XRGB8888),
            Arg::U(0),
        ];
        c.send(params, PARAMS_CREATE_IMMED, &args)?;
        c.send(params, PARAMS_DESTROY, &[])?;
        win.claim(buffer);
        Ok(buffer)
    }

    /// Show slot `i` (drawn and finished) from the next compositor frame.
    pub fn present<S>(&mut self, win: &mut Window<S>, i: usize) -> Result<(), Error> {
        let s = &mut self.slots[i];
        s.busy = true;
        let (w, h) = (s.target.w as i32, s.target.h as i32);
        let buffer = s.buffer;
        let c = win.conn();
        c.send(
            self.surface,
            SURFACE_ATTACH,
            &[Arg::U(buffer), Arg::I(0), Arg::I(0)],
        )?;
        c.send(
            self.surface,
            SURFACE_DAMAGE_BUFFER,
            &[0, 0, w, h].map(Arg::I),
        )?;
        let cb = c.new_id();
        c.send(self.surface, SURFACE_FRAME, &[Arg::U(cb)])?;
        c.send(self.surface, SURFACE_COMMIT, &[])?;
        win.claim(cb);
        self.frame_cbs.push(cb);
        self.shown = true;
        Ok(())
    }

    /// One of our objects' events. True when a presented frame reached the screen.
    pub fn on_event<S>(&mut self, win: &mut Window<S>, ev: &WlEvent) -> Result<bool, Error> {
        if ev.obj == self.dmabuf {
            if ev.op == EV_DMABUF_MODIFIER {
                let mut a = ev.args();
                let (format, hi, lo) = (a.u32()?, a.u32()?, a.u32()?);
                self.modifiers.extend(modifier(format, hi, lo));
            }
            return Ok(false);
        }
        if ev.op != EV_RELEASE_OR_DONE {
            return Ok(false);
        }
        if let Some(i) = self.frame_cbs.iter().position(|&cb| cb == ev.obj) {
            self.frame_cbs.remove(i);
            return Ok(true);
        }
        let Some(i) = self.slots.iter().position(|s| s.buffer == ev.obj) else {
            return Ok(false);
        };
        let s = &mut self.slots[i];
        s.busy = false;
        if (s.target.w, s.target.h) != self.size {
            let s = self.slots.remove(i);
            win.conn().send(s.buffer, BUFFER_DESTROY, &[])?;
        }
        Ok(false)
    }
}

/// A modifier event's modifier, if it is for our format and an explicit one.
fn modifier(format: u32, hi: u32, lo: u32) -> Option<u64> {
    let m = (u64::from(hi) << 32) | u64::from(lo);
    (format == GBM_FORMAT_XRGB8888 && m != MOD_INVALID).then_some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_splits_into_hi_and_lo() {
        let m = MOD_INVALID;
        assert_eq!(((m >> 32) as u32, m as u32), (0x00ff_ffff, 0xffff_ffff));
    }

    #[test]
    fn modifier_events_keep_only_explicit_xrgb_ones() {
        // AMD_FMT_MOD-style value: vendor 0x02 in the top byte.
        assert_eq!(
            modifier(GBM_FORMAT_XRGB8888, 0x0200_0000, 0x0000_1234),
            Some(0x0200_0000_0000_1234)
        );
        assert_eq!(modifier(GBM_FORMAT_XRGB8888, 0, 0), Some(0)); // LINEAR
        assert_eq!(
            modifier(GBM_FORMAT_XRGB8888, 0x00ff_ffff, 0xffff_ffff),
            None
        );
        assert_eq!(modifier(u32::from_le_bytes(*b"AR24"), 0, 0), None);
    }
}
