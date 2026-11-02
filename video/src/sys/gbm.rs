//! Raw libgbm: GPU buffers we render into and hand to the compositor as dmabufs.
//! Signatures from /usr/include/gbm.h (mesa 26.2).

use std::ffi::{c_int, c_uint};

#[repr(C)]
pub struct gbm_device {
    _p: [u8; 0],
}

#[repr(C)]
pub struct gbm_bo {
    _p: [u8; 0],
}

/// fourcc('X','R','2','4'): 32-bit RGB, alpha ignored (also the DRM/Wayland code).
pub const GBM_FORMAT_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
pub const GBM_BO_USE_RENDERING: u32 = 1 << 2;

#[link(name = "gbm")]
unsafe extern "C" {
    pub fn gbm_create_device(fd: c_int) -> *mut gbm_device;
    pub fn gbm_device_destroy(gbm: *mut gbm_device);
    pub fn gbm_bo_create(
        gbm: *mut gbm_device,
        width: u32,
        height: u32,
        format: u32,
        flags: u32,
    ) -> *mut gbm_bo;
    /// gbm.h:372 (mesa 26.2): like gbm_bo_create, with the modifiers it may pick from.
    pub fn gbm_bo_create_with_modifiers2(
        gbm: *mut gbm_device,
        width: u32,
        height: u32,
        format: u32,
        modifiers: *const u64,
        count: c_uint,
        flags: u32,
    ) -> *mut gbm_bo;
    pub fn gbm_bo_get_plane_count(bo: *mut gbm_bo) -> c_int;
    pub fn gbm_bo_get_fd_for_plane(bo: *mut gbm_bo, plane: c_int) -> c_int;
    pub fn gbm_bo_get_stride_for_plane(bo: *mut gbm_bo, plane: c_int) -> u32;
    pub fn gbm_bo_get_offset(bo: *mut gbm_bo, plane: c_int) -> u32;
    pub fn gbm_bo_get_modifier(bo: *mut gbm_bo) -> u64;
    pub fn gbm_bo_destroy(bo: *mut gbm_bo);
}

#[cfg(test)]
mod tests {
    #[test]
    fn xrgb_fourcc_matches_gbm_h() {
        assert_eq!(super::GBM_FORMAT_XRGB8888, 0x3432_5258);
    }
}
