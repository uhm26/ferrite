//! Opcodes and enum values we use. Requests and events are numbered in XML order.
//! Sources: /usr/share/wayland/wayland.xml (wayland 1.26) and
//! wlr-layer-shell-unstable-v1.xml (wlr-protocols, layer shell v5).

pub mod display {
    pub const SYNC: u16 = 0;
    pub const GET_REGISTRY: u16 = 1;
    pub const EV_ERROR: u16 = 0;
    pub const EV_DELETE_ID: u16 = 1;
}

pub mod registry {
    pub const BIND: u16 = 0;
    pub const EV_GLOBAL: u16 = 0;
    pub const EV_GLOBAL_REMOVE: u16 = 1;
}

pub mod compositor {
    pub const CREATE_SURFACE: u16 = 0;
    pub const CREATE_REGION: u16 = 1;
}

pub mod shm {
    pub const CREATE_POOL: u16 = 0;
    pub const FORMAT_XRGB8888: u32 = 1;
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
    pub const DESTROY: u16 = 0;
    pub const ATTACH: u16 = 1;
    pub const SET_OPAQUE_REGION: u16 = 4;
    pub const COMMIT: u16 = 6;
    pub const SET_BUFFER_SCALE: u16 = 8; // since v3
    pub const DAMAGE_BUFFER: u16 = 9; // since v4
}

pub mod region {
    pub const DESTROY: u16 = 0;
    pub const ADD: u16 = 1;
}

pub mod output {
    pub const RELEASE: u16 = 0; // since v3
    pub const EV_SCALE: u16 = 3; // since v2
    pub const EV_DONE: u16 = 2; // since v2
}

pub mod layer_shell {
    pub const GET_LAYER_SURFACE: u16 = 0;
    pub const LAYER_BACKGROUND: u32 = 0;
}

pub mod layer_surface {
    pub const SET_ANCHOR: u16 = 1;
    pub const SET_EXCLUSIVE_ZONE: u16 = 2;
    pub const ACK_CONFIGURE: u16 = 6;
    pub const DESTROY: u16 = 7;
    pub const EV_CONFIGURE: u16 = 0;
    pub const EV_CLOSED: u16 = 1;
    /// top | bottom | left | right
    pub const ANCHOR_ALL: u32 = 1 | 2 | 4 | 8;
}

/// wl_display is always object 1.
pub const DISPLAY: u32 = 1;
