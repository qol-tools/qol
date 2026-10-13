mod default_output;
mod identity;
mod meter;
mod mute;
mod volume;

pub(crate) use default_output::{effective_default, set_default_output};
pub(crate) use identity::{
    companion_input, identity_for_node, list_audio_devices, resolve_audio_device,
};
pub(crate) use meter::Meter;
pub(crate) use mute::{is_muted, set_muted};
pub(crate) use volume::{set_volume_percent, volume_percent};

use crate::bluetooth::BluetoothEndpoint;
use crate::devices::{Device, Direction};
use crate::AudioError;

pub(crate) fn list_devices(_direction: Direction) -> Result<Vec<Device>, AudioError> {
    Err(AudioError::Unsupported)
}

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
