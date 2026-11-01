//! Pixel buffers: one shared-memory pool with three slots, reused across resizes.
//! A resize makes new wl_buffers over the same memory (cheap: no new file, no fresh
//! pages to fault in, nothing new for the compositor to map); the pool is only
//! replaced when the window outgrows a slot. A buffer is busy from commit until the
//! compositor releases it; each remembers what it shows so the renderer only redraws
//! what changed since that buffer was last drawn.

use super::Window;
use crate::render::Shown;
use crate::render::draw::Canvas;
use crate::sys::mem::Pixels;
use crate::wl::proto::{buffer, shm_pool};
use crate::wl::{Error, Event as WlEvent};

const SLOTS: usize = 3;

pub(super) struct Pool {
    pixels: Pixels,
    id: u32,
    slot_bytes: usize,
    /// Bumped per pool: buffers of an older pool are destroyed once released.
    generation: u32,
}

pub(super) struct Buffer {
    id: u32,
    slot: usize,
    generation: u32,
    w: usize,
    h: usize,
    pub(super) busy: bool,
    shown: Shown,
}

impl Buffer {
    pub(super) fn wl_id(&self) -> u32 {
        self.id
    }
}

impl Window {
    /// Index of a free buffer of the current size, making one if needed. None if
    /// every slot is busy (the caller tries again on the next release).
    pub fn acquire(&mut self) -> Result<Option<usize>, Error> {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return Ok(None);
        }
        if self.pool.as_ref().is_none_or(|p| p.slot_bytes < w * h * 4) {
            self.new_pool(w * h * 4)?;
        }
        let generation = self.pool.as_ref().map_or(0, |p| p.generation);
        let fits = |b: &Buffer| (b.w, b.h, b.generation) == (w, h, generation);
        // Free buffers of another size or pool are useless now.
        let mut i = 0;
        while i < self.buffers.len() {
            if !self.buffers[i].busy && !fits(&self.buffers[i]) {
                let b = self.buffers.swap_remove(i);
                self.conn.send(b.id, buffer::DESTROY, &[])?;
            } else {
                i += 1;
            }
        }
        if let Some(i) = self.buffers.iter().position(|b| !b.busy && fits(b)) {
            return Ok(Some(i));
        }
        let taken = |s: usize| {
            self.buffers
                .iter()
                .any(|b| b.slot == s && b.generation == generation)
        };
        let Some(slot) = (0..SLOTS).find(|&s| !taken(s)) else {
            return Ok(None);
        };
        let pool = self.pool.as_ref().map_or(0, |p| p.id);
        let bytes = self.pool.as_ref().map_or(0, |p| p.slot_bytes);
        let id = self
            .conn
            .pool_buffer(pool, slot * bytes, w as i32, h as i32)?;
        self.buffers.push(Buffer {
            id,
            slot,
            generation,
            w,
            h,
            busy: false,
            shown: Shown::default(),
        });
        Ok(Some(self.buffers.len() - 1))
    }

    /// A pool with room for `bytes` per slot plus headroom, so growing the window a
    /// little doesn't need a new one.
    fn new_pool(&mut self, bytes: usize) -> Result<(), Error> {
        let slot_bytes = (bytes * 3 / 2).next_multiple_of(4096);
        let pixels = Pixels::new(slot_bytes * SLOTS)?;
        let id = self
            .conn
            .create_pool(self.ids.shm, pixels.fd(), slot_bytes * SLOTS)?;
        let generation = self.pool.as_ref().map_or(0, |p| p.generation + 1);
        if let Some(old) = self.pool.replace(Pool {
            pixels,
            id,
            slot_bytes,
            generation,
        }) {
            // Buffers made from it stay valid (wl_shm_pool.destroy); we never draw
            // into them again, so our mapping can go.
            self.conn.send(old.id, shm_pool::DESTROY, &[])?;
        }
        Ok(())
    }

    /// The pixels of buffer `i` to draw into, and what it shows now.
    pub fn canvas(&mut self, i: usize) -> (Canvas<'_>, &mut Shown) {
        let b = &mut self.buffers[i];
        let (w, h) = (b.w, b.h);
        let px = match &mut self.pool {
            Some(p) => {
                let start = b.slot * p.slot_bytes / 4;
                &mut p.pixels.as_u32_mut()[start..start + w * h]
            }
            None => &mut [][..],
        };
        (Canvas { px, w, h }, &mut b.shown)
    }

    /// Every buffer redraws fully next time (palette change, font change).
    pub fn invalidate_buffers(&mut self) {
        for b in &mut self.buffers {
            b.shown.invalidate();
        }
    }

    /// Bytes of pixel memory in use by our buffers.
    pub fn buffer_bytes(&self) -> usize {
        self.buffers.iter().map(|b| b.w * b.h * 4).sum()
    }

    /// wl_buffer.release: true if it was one of ours. A released buffer from an old
    /// pool or of an old size is destroyed right away.
    pub(super) fn on_buffer(&mut self, ev: &WlEvent) -> bool {
        if ev.op != buffer::EV_RELEASE {
            return false;
        }
        let Some(i) = self.buffers.iter().position(|b| b.id == ev.obj) else {
            return false;
        };
        let generation = self.pool.as_ref().map_or(0, |p| p.generation);
        let b = &mut self.buffers[i];
        b.busy = false;
        if (b.w, b.h, b.generation) != (self.width, self.height, generation) {
            let b = self.buffers.swap_remove(i);
            let _ = self.conn.send(b.id, buffer::DESTROY, &[]);
        }
        true
    }
}
