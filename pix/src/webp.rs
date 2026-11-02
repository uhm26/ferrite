//! WebP (still, animated, lossy, lossless) through the system libwebp: 2× faster than the
//! pure-Rust decoder on our files.
//! libwebp's animation decoder hands back whole composited canvases, stills included.

use std::ffi::c_int;

use crate::decode::{Error, Frame, MAX_FRAME_BYTES, delay_ms, to_rgb};
use crate::sys::webp::*;
use zune_core::colorspace::ColorSpace;

/// Owns the decoder; deletes it on drop. Borrows `bytes` (libwebp keeps pointers into them).
struct Decoder<'a>(*mut WebPAnimDecoder, std::marker::PhantomData<&'a [u8]>);

impl Drop for Decoder<'_> {
    fn drop(&mut self) {
        // SAFETY: the pointer came from WebPAnimDecoderNewInternal and is deleted once.
        unsafe { WebPAnimDecoderDelete(self.0) }
    }
}

pub fn frames(bytes: &[u8], bg: [u8; 3]) -> Result<Vec<Frame>, Error> {
    let bad = || Error::Decode("bad WebP".into());
    let mut opts = WebPAnimDecoderOptions::default();
    // SAFETY: `opts` is a valid, writable options struct of the header's layout.
    if unsafe { WebPAnimDecoderOptionsInitInternal(&mut opts, WEBP_DEMUX_ABI_VERSION) } == 0 {
        return Err(Error::Decode("libwebp ABI mismatch".into()));
    }
    (opts.color_mode, opts.use_threads) = (MODE_RGBA, 1);
    let data = WebPData {
        bytes: bytes.as_ptr(),
        size: bytes.len(),
    };
    // SAFETY: `data` points at `bytes`, which outlive the decoder (lifetime on `Decoder`).
    let raw = unsafe { WebPAnimDecoderNewInternal(&data, &opts, WEBP_DEMUX_ABI_VERSION) };
    if raw.is_null() {
        return Err(bad());
    }
    let dec = Decoder(raw, std::marker::PhantomData);
    let mut info = WebPAnimInfo::default();
    // SAFETY: `dec.0` is a live decoder, `info` a writable struct of the header's layout.
    if unsafe { WebPAnimDecoderGetInfo(dec.0, &mut info) } == 0 {
        return Err(bad());
    }
    let (w, h) = (info.canvas_width as usize, info.canvas_height as usize);
    if w == 0 || h == 0 {
        return Err(bad());
    }
    let mut out = Vec::new();
    let (mut buf, mut ts, mut prev): (*mut u8, c_int, c_int) = (std::ptr::null_mut(), 0, 0);
    // shortcut: frames past MAX_FRAME_BYTES are dropped (the animation loops early).
    while out.len() * w * h * 3 < MAX_FRAME_BYTES
        // SAFETY: live decoder, writable out-params; on success `buf` holds w*h*4 bytes
        // valid until the next call, and we copy them out before that.
        && unsafe { WebPAnimDecoderGetNext(dec.0, &mut buf, &mut ts) } != 0
    {
        // SAFETY: see above: `buf` points at w*h*4 initialized bytes.
        let rgba = unsafe { std::slice::from_raw_parts(buf, w * h * 4) }.to_vec();
        let ms = (ts - prev).max(0) as u32;
        prev = ts;
        out.push(Frame {
            img: to_rgb(rgba, (w, h), ColorSpace::RGBA, bg)?,
            ms: delay_ms(ms),
        });
    }
    if out.is_empty() { Err(bad()) } else { Ok(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_lossless_pixel_and_rejects_junk() {
        // The well-known 1×1 lossless WebP browsers use for feature checks (an opaque pixel).
        let one = [
            82, 73, 70, 70, 26, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 76, 13, 0, 0, 0, 47, 0, 0, 0,
            16, 7, 16, 17, 17, 136, 136, 254, 7, 0,
        ];
        let f = frames(&one, [9, 9, 9]).unwrap();
        assert_eq!((f.len(), f[0].img.w, f[0].img.h, f[0].ms), (1, 1, 1, 100));
        assert!(frames(b"RIFF\0\0\0\0WEBPjunk", [0; 3]).is_err());
    }
}
