//! Opcodes and enum values we use. Requests and events are numbered in XML order.
//! Sources: /usr/share/wayland/wayland.xml (wayland 1.26),
//! /usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml and
//! staging/cursor-shape/cursor-shape-v1.xml. Started from ../wall/src/wl/proto.rs.

/// wl_display is always object 1.
pub const DISPLAY: u32 = 1;

pub mod display {
    pub const SYNC: u16 = 0;
    pub const GET_REGISTRY: u16 = 1;
    pub const EV_ERROR: u16 = 0;
    pub const EV_DELETE_ID: u16 = 1;
}

pub mod registry {
    pub const BIND: u16 = 0;
    pub const EV_GLOBAL: u16 = 0;
}

pub mod callback {
    pub const EV_DONE: u16 = 0;
}

pub mod compositor {
    pub const CREATE_SURFACE: u16 = 0;
    pub const CREATE_REGION: u16 = 1;
}

pub mod shm {
    pub const CREATE_POOL: u16 = 0;
    pub const FORMAT_ARGB8888: u32 = 0;
}

pub mod shm_pool {
    pub const CREATE_BUFFER: u16 = 0;
    pub const DESTROY: u16 = 1;
}

pub mod buffer {
    pub const DESTROY: u16 = 0;
    pub const EV_RELEASE: u16 = 0;
}

pub mod surface {
    pub const ATTACH: u16 = 1;
    pub const FRAME: u16 = 3;
    pub const SET_OPAQUE_REGION: u16 = 4;
    pub const COMMIT: u16 = 6;
    pub const DAMAGE_BUFFER: u16 = 9; // since v4
}

pub mod region {
    pub const DESTROY: u16 = 0;
    pub const ADD: u16 = 1;
}

pub mod seat {
    pub const GET_POINTER: u16 = 0;
    pub const GET_KEYBOARD: u16 = 1;
    pub const EV_CAPABILITIES: u16 = 0;
    pub const CAP_POINTER: u32 = 1;
    pub const CAP_KEYBOARD: u32 = 2;
}

pub mod keyboard {
    pub const EV_KEYMAP: u16 = 0;
    pub const EV_ENTER: u16 = 1;
    pub const EV_LEAVE: u16 = 2;
    pub const EV_KEY: u16 = 3;
    pub const EV_MODIFIERS: u16 = 4;
    pub const EV_REPEAT_INFO: u16 = 5; // since v4
    pub const KEYMAP_XKB_V1: u32 = 1;
}

pub mod pointer {
    pub const EV_ENTER: u16 = 0;
    pub const EV_LEAVE: u16 = 1;
    pub const EV_MOTION: u16 = 2;
    pub const EV_BUTTON: u16 = 3;
    pub const EV_AXIS: u16 = 4;
    pub const EV_FRAME: u16 = 5; // since v5
    pub const EV_AXIS_DISCRETE: u16 = 8; // v5 to v7
    pub const AXIS_VERTICAL: u32 = 0;
    pub const BTN_LEFT: u32 = 0x110; // linux/input-event-codes.h
    pub const BTN_RIGHT: u32 = 0x111;
    pub const BTN_MIDDLE: u32 = 0x112;
}

pub mod data_device_manager {
    pub const CREATE_DATA_SOURCE: u16 = 0;
    pub const GET_DATA_DEVICE: u16 = 1;
}

pub mod data_device {
    pub const SET_SELECTION: u16 = 1;
    pub const EV_DATA_OFFER: u16 = 0;
    pub const EV_SELECTION: u16 = 5;
}

pub mod data_offer {
    pub const RECEIVE: u16 = 1;
    pub const DESTROY: u16 = 2;
    pub const EV_OFFER: u16 = 0;
}

pub mod data_source {
    pub const OFFER: u16 = 0;
    pub const DESTROY: u16 = 1;
    pub const EV_SEND: u16 = 1;
    pub const EV_CANCELLED: u16 = 2;
}

pub mod wm_base {
    pub const GET_XDG_SURFACE: u16 = 2;
    pub const PONG: u16 = 3;
    pub const EV_PING: u16 = 0;
}

pub mod xdg_surface {
    pub const GET_TOPLEVEL: u16 = 1;
    pub const ACK_CONFIGURE: u16 = 4;
    pub const EV_CONFIGURE: u16 = 0;
}

pub mod toplevel {
    pub const SET_TITLE: u16 = 2;
    pub const SET_APP_ID: u16 = 3;
    pub const EV_CONFIGURE: u16 = 0;
    pub const EV_CLOSE: u16 = 1;
}

pub mod cursor_shape {
    /// wp_cursor_shape_manager_v1.get_pointer
    pub const GET_POINTER: u16 = 1;
    /// wp_cursor_shape_device_v1.set_shape
    pub const SET_SHAPE: u16 = 1;
    pub const SHAPE_TEXT: u32 = 9;
}
