use crate::default_output::{DefaultSnapshot, ReleaseCapability};
use crate::devices::{Direction, Identity};
use crate::AudioError;

pub(crate) fn effective_default(direction: Direction) -> Result<Option<String>, AudioError> {
    let _ = direction;
    Err(AudioError::Unsupported)
}

pub(crate) fn capture_default(direction: Direction) -> Result<DefaultSnapshot, AudioError> {
    let _ = direction;
    Err(AudioError::Unsupported)
}

pub(crate) fn default_release_capability(
    direction: Direction,
) -> Result<ReleaseCapability, AudioError> {
    let _ = direction;
    Err(AudioError::Unsupported)
}

pub(crate) fn set_default_output(
    direction: Direction,
    output: &Identity,
) -> Result<(), AudioError> {
    let _ = (direction, output);
    Err(AudioError::Unsupported)
}

pub(crate) fn restore_default(snapshot: &DefaultSnapshot) -> Result<(), AudioError> {
    let _ = snapshot;
    Err(AudioError::Unsupported)
}
