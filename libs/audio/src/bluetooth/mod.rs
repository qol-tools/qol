use crate::devices::Direction;
use crate::platform;
use crate::AudioError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothEndpoint {
    pub address: String,
    pub direction: Direction,
    pub active: bool,
}

pub fn endpoints() -> Result<Vec<BluetoothEndpoint>, AudioError> {
    platform::bluetooth_endpoints()
}

pub fn reconnect(address: &str) -> Result<usize, AudioError> {
    platform::reconnect_bluetooth(address)
}

pub fn disconnect(address: &str) -> Result<usize, AudioError> {
    platform::disconnect_bluetooth(address)
}
