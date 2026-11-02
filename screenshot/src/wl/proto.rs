//! Opcodes we use. Requests and events are numbered in XML order. Sources:
//! /usr/share/wayland/wayland.xml and wlr-protocols
//! unstable/wlr-screencopy-unstable-v1.xml (version 3).

/// wl_display is always object 1.
pub const DISPLAY: u32 = 1;

pub mod display {
    pub const SYNC: u16 = 0;
    pub const GET_REGISTRY: u16 = 1;
    pub const EV_ERROR: u16 = 0;
}

pub mod registry {
    pub const BIND: u16 = 0;
    pub const EV_GLOBAL: u16 = 0;
}

pub mod callback {
    pub const EV_DONE: u16 = 0;
}

pub mod shm {
    pub const NAME: &str = "wl_shm";
    pub const CREATE_POOL: u16 = 0;
}

pub mod shm_pool {
    pub const CREATE_BUFFER: u16 = 0;
    pub const DESTROY: u16 = 1;
}

pub mod output {
    pub const NAME: &str = "wl_output";
    pub const EV_GEOMETRY: u16 = 0;
    pub const EV_NAME: u16 = 4;
}

pub mod screencopy {
    pub const NAME: &str = "zwlr_screencopy_manager_v1";
    pub const CAPTURE_OUTPUT: u16 = 0;
}

/// zwlr_screencopy_frame_v1 and hyprland_toplevel_export_frame_v1
pub mod frame {
    pub const COPY: u16 = 0;
    /// Same number for screencopy and toplevel export frames.
    pub const DESTROY: u16 = 1;
    // Event numbers differ per protocol: see `Kind` in src/capture/frame.rs.
    /// flags: the image is upside down.
    pub const Y_INVERT: u32 = 1;
}

// ---- region/window select: sources are wayland.xml, wlr-layer-shell-unstable-v1.xml,
// viewporter.xml, single-pixel-buffer-v1.xml, cursor-shape-v1.xml,
// hyprland-toplevel-export-v1.xml (hyprwm/hyprland-protocols), ext-data-control-v1.xml.

pub mod compositor {
    pub const NAME: &str = "wl_compositor";
    pub const CREATE_SURFACE: u16 = 0;
    pub const CREATE_REGION: u16 = 1;
}

pub mod surface {
    pub const DESTROY: u16 = 0;
    pub const ATTACH: u16 = 1;
    pub const DAMAGE: u16 = 2;
    pub const FRAME: u16 = 3;
    pub const SET_INPUT_REGION: u16 = 5;
    pub const COMMIT: u16 = 6;
    pub const SET_BUFFER_TRANSFORM: u16 = 7;
    /// wl_output.transform: flipped_180 = upside down.
    pub const FLIPPED_180: i32 = 6;
}

pub mod subcompositor {
    pub const NAME: &str = "wl_subcompositor";
    pub const GET_SUBSURFACE: u16 = 1;
}

pub mod subsurface {
    pub const SET_POSITION: u16 = 1;
}

pub mod seat {
    pub const NAME: &str = "wl_seat";
    pub const GET_POINTER: u16 = 0;
    pub const GET_KEYBOARD: u16 = 1;
}

pub mod pointer {
    pub const RELEASE: u16 = 1;
    pub const EV_ENTER: u16 = 0;
    pub const EV_MOTION: u16 = 2;
    pub const EV_BUTTON: u16 = 3;
    /// linux/input-event-codes.h
    pub const BTN_LEFT: u32 = 0x110;
    pub const BTN_RIGHT: u32 = 0x111;
}

pub mod keyboard {
    pub const RELEASE: u16 = 0;
    pub const EV_KEYMAP: u16 = 0;
    pub const EV_KEY: u16 = 3;
    /// linux/input-event-codes.h (evdev codes, no xkb needed)
    pub const KEY_ESC: u32 = 1;
}

pub mod layer_shell {
    pub const NAME: &str = "zwlr_layer_shell_v1";
    pub const GET_LAYER_SURFACE: u16 = 0;
    pub const LAYER_OVERLAY: u32 = 3;
}

/// zwlr_layer_surface_v1
pub mod layer_surface {
    pub const SET_ANCHOR: u16 = 1;
    pub const SET_EXCLUSIVE_ZONE: u16 = 2;
    pub const SET_KEYBOARD_INTERACTIVITY: u16 = 4;
    pub const ACK_CONFIGURE: u16 = 6;
    pub const DESTROY: u16 = 7;
    pub const EV_CONFIGURE: u16 = 0;
    pub const EV_CLOSED: u16 = 1;
    /// top | bottom | left | right
    pub const ANCHOR_ALL: u32 = 15;
    pub const KEYBOARD_EXCLUSIVE: u32 = 1;
}

pub mod viewporter {
    pub const NAME: &str = "wp_viewporter";
    pub const GET_VIEWPORT: u16 = 1;
}

pub mod viewport {
    pub const SET_DESTINATION: u16 = 2;
}

pub mod single_pixel {
    pub const NAME: &str = "wp_single_pixel_buffer_manager_v1";
    pub const CREATE_U32_RGBA_BUFFER: u16 = 1;
}

pub mod cursor_shape {
    pub const NAME: &str = "wp_cursor_shape_manager_v1";
    pub const GET_POINTER: u16 = 1;
    /// wp_cursor_shape_device_v1
    pub const DESTROY_DEVICE: u16 = 0;
    pub const SET_SHAPE: u16 = 1;
    pub const CROSSHAIR: u32 = 8;
}

/// hyprland_toplevel_export_manager_v1 and its frames. Frame events are numbered
/// differently from screencopy's: flags 2, ready 3, failed 4.
pub mod toplevel_export {
    pub const NAME: &str = "hyprland_toplevel_export_manager_v1";
    pub const CAPTURE_TOPLEVEL: u16 = 0;
}

/// *_data_control_manager_v1 (ext and wlr use the same numbers).
pub mod data_control {
    pub const EXT: &str = "ext_data_control_manager_v1";
    pub const WLR: &str = "zwlr_data_control_manager_v1";
    pub const CREATE_DATA_SOURCE: u16 = 0;
    pub const GET_DATA_DEVICE: u16 = 1;
    /// *_data_control_device_v1
    pub const SET_SELECTION: u16 = 0;
    /// *_data_control_source_v1
    pub const OFFER: u16 = 0;
    pub const EV_SEND: u16 = 0;
    pub const EV_CANCELLED: u16 = 1;
}
