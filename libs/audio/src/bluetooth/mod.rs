use crate::devices::Direction;

pub use crate::platform::{
    bluetooth_endpoints as endpoints, disconnect_bluetooth as disconnect,
    reconnect_bluetooth as reconnect,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothEndpoint {
    pub address: String,
    pub direction: Direction,
    pub active: bool,
}
