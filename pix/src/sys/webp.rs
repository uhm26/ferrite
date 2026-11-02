//! libwebpdemux's animation decoder (also decodes stills): /usr/include/webp/demux.h,
//! mux_types.h, decode.h (libwebp 1.6.0).

use std::ffi::c_int;

/// demux.h: `#define WEBP_DEMUX_ABI_VERSION 0x0107`; the `*Internal` entry points check it.
pub const WEBP_DEMUX_ABI_VERSION: c_int = 0x0107;
/// decode.h: `MODE_RGBA = 1` (WEBP_CSP_MODE).
pub const MODE_RGBA: c_int = 1;

#[repr(C)]
pub struct WebPAnimDecoder {
    _p: [u8; 0],
}

/// mux_types.h: `struct WebPData { const uint8_t* bytes; size_t size; }`
#[repr(C)]
pub struct WebPData {
    pub bytes: *const u8,
    pub size: usize,
}

/// demux.h: `struct WebPAnimDecoderOptions { WEBP_CSP_MODE color_mode; int use_threads; uint32_t padding[7]; }`
#[repr(C)]
#[derive(Default)]
pub struct WebPAnimDecoderOptions {
    pub color_mode: c_int,
    pub use_threads: c_int,
    pub padding: [u32; 7],
}

/// demux.h: `struct WebPAnimInfo { uint32_t canvas_width, canvas_height, loop_count, bgcolor, frame_count; uint32_t pad[4]; }`
#[repr(C)]
#[derive(Default)]
pub struct WebPAnimInfo {
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub loop_count: u32,
    pub bgcolor: u32,
    pub frame_count: u32,
    pub pad: [u32; 4],
}

#[link(name = "webpdemux")]
unsafe extern "C" {
    // int WebPAnimDecoderOptionsInitInternal(WebPAnimDecoderOptions*, int);
    pub fn WebPAnimDecoderOptionsInitInternal(o: *mut WebPAnimDecoderOptions, abi: c_int) -> c_int;
    // WebPAnimDecoder* WebPAnimDecoderNewInternal(const WebPData*, const WebPAnimDecoderOptions*, int);
    pub fn WebPAnimDecoderNewInternal(
        data: *const WebPData,
        o: *const WebPAnimDecoderOptions,
        abi: c_int,
    ) -> *mut WebPAnimDecoder;
    // int WebPAnimDecoderGetInfo(const WebPAnimDecoder* dec, WebPAnimInfo* info);
    pub fn WebPAnimDecoderGetInfo(d: *const WebPAnimDecoder, info: *mut WebPAnimInfo) -> c_int;
    // int WebPAnimDecoderGetNext(WebPAnimDecoder* dec, uint8_t** buf, int* timestamp);
    pub fn WebPAnimDecoderGetNext(d: *mut WebPAnimDecoder, buf: *mut *mut u8, ts: *mut c_int) -> c_int;
    // void WebPAnimDecoderDelete(WebPAnimDecoder* dec);
    pub fn WebPAnimDecoderDelete(d: *mut WebPAnimDecoder);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structs_match_the_header_sizes() {
        assert_eq!(size_of::<WebPData>(), 16);
        assert_eq!(size_of::<WebPAnimDecoderOptions>(), 36);
        assert_eq!(size_of::<WebPAnimInfo>(), 36);
    }
}
