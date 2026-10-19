use crate::bluetooth::BluetoothEndpoint;
use crate::AudioError;

pub fn bluetooth_endpoints() -> Result<Vec<BluetoothEndpoint>, AudioError> {
    Err(AudioError::Unsupported)
}

pub fn reconnect_bluetooth(address: &str) -> Result<usize, AudioError> {
    let _ = address;
    Err(AudioError::Unsupported)
}

pub fn disconnect_bluetooth(address: &str) -> Result<usize, AudioError> {
    let _ = address;
    Err(AudioError::Unsupported)
}
