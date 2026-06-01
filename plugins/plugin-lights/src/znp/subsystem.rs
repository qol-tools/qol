pub const SYS: u8 = 0x01;
pub const AF: u8 = 0x04;
pub const ZDO: u8 = 0x05;
pub const SAPI: u8 = 0x06;
pub const UTIL: u8 = 0x07;

pub mod sys {
    pub const RESET_REQ: u8 = 0x00;
    pub const PING: u8 = 0x01;
    pub const OSAL_NV_WRITE: u8 = 0x09;
    pub const RESET_IND: u8 = 0x80;
}

pub mod af {
    pub const REGISTER: u8 = 0x00;
    pub const DATA_REQUEST: u8 = 0x01;
    pub const DATA_CONFIRM: u8 = 0x80;
    pub const INCOMING_MSG: u8 = 0x81;
}

pub mod zdo {
    pub const NWK_ADDR_REQ: u8 = 0x00;
    pub const NWK_ADDR_RSP: u8 = 0x80;
    pub const SIMPLE_DESC_REQ: u8 = 0x04;
    pub const SIMPLE_DESC_RSP: u8 = 0x84;
    pub const ACTIVE_EP_REQ: u8 = 0x05;
    pub const ACTIVE_EP_RSP: u8 = 0x85;
    pub const MGMT_PERMIT_JOIN_REQ: u8 = 0x36;
    pub const MGMT_PERMIT_JOIN_RSP: u8 = 0xB6;
    pub const STARTUP_FROM_APP: u8 = 0x40;
    pub const STATE_CHANGE_IND: u8 = 0xC0;
    pub const END_DEVICE_ANNCE_IND: u8 = 0xC1;
}

pub mod sapi {
    pub const ZB_START_REQUEST: u8 = 0x00;
    pub const ZB_START_CONFIRM: u8 = 0x80;
    pub const ZB_WRITE_CONFIGURATION: u8 = 0x05;
    pub const ZB_READ_CONFIGURATION: u8 = 0x04;
}

pub mod util {
    pub const GET_DEVICE_INFO: u8 = 0x00;
}

pub mod nv_id {
    pub const LOGICAL_TYPE: u16 = 0x0087;
    pub const PAN_ID: u16 = 0x0083;
    pub const CHANLIST: u16 = 0x0084;
    pub const PRECFGKEY: u16 = 0x0062;
    pub const PRECFGKEYS_ENABLE: u16 = 0x0063;
    pub const ZDO_DIRECT_CB: u16 = 0x008F;
}

pub const COORDINATOR: u8 = 0x00;
