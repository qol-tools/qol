use crate::devices::Direction;
use crate::AudioError;

pub(crate) fn is_muted(direction: Direction) -> Result<Option<bool>, AudioError> {
    let _ = direction;
    Err(AudioError::Unsupported)
}

pub(crate) fn set_muted(direction: Direction, muted: bool) -> Result<(), AudioError> {
    let _ = (direction, muted);
    Err(AudioError::Unsupported)
}
