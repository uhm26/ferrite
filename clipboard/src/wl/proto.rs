//! Opcodes we use. Requests and events are numbered in XML order. Sources:
//! /usr/share/wayland/wayland.xml and
//! /usr/share/wayland-protocols/staging/ext-data-control/ext-data-control-v1.xml.
//! zwlr_data_control_*_v1 (wlr-data-control-unstable-v1.xml) uses the same numbers.

/// wl_display is always object 1.
pub const DISPLAY: u32 = 1;
/// Ids from here up are created by the compositor (data offers).
pub const SERVER_ID_START: u32 = 0xff00_0000;

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

/// *_data_control_manager_v1
pub mod manager {
    pub const EXT: &str = "ext_data_control_manager_v1";
    pub const WLR: &str = "zwlr_data_control_manager_v1";
    pub const CREATE_DATA_SOURCE: u16 = 0;
    pub const GET_DATA_DEVICE: u16 = 1;
}

/// *_data_control_device_v1
pub mod device {
    pub const SET_SELECTION: u16 = 0;
    pub const EV_DATA_OFFER: u16 = 0;
    pub const EV_SELECTION: u16 = 1;
    pub const EV_FINISHED: u16 = 2;
    pub const EV_PRIMARY_SELECTION: u16 = 3;
}

/// *_data_control_source_v1
pub mod source {
    pub const OFFER: u16 = 0;
    pub const DESTROY: u16 = 1;
    pub const EV_SEND: u16 = 0;
    pub const EV_CANCELLED: u16 = 1;
}

/// *_data_control_offer_v1
pub mod offer {
    pub const RECEIVE: u16 = 0;
    pub const DESTROY: u16 = 1;
    pub const EV_OFFER: u16 = 0;
}
