use crate::devices::{AudioDevice, Direction, Identity, Resolution};
use crate::AudioError;

pub(crate) fn list_audio_devices(direction: Direction) -> Result<Vec<AudioDevice>, AudioError> {
    let _ = direction;
    Err(AudioError::Unsupported)
}

pub(crate) fn resolve_audio_device(
    direction: Direction,
    requested: &str,
) -> Result<Resolution, AudioError> {
    let _ = (direction, requested);
    Err(AudioError::Unsupported)
}

pub(crate) fn identity_for_node(
    direction: Direction,
    node: &str,
) -> Result<Option<Identity>, AudioError> {
    let _ = (direction, node);
    Err(AudioError::Unsupported)
}
