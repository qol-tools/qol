use crate::devices::Direction;
use crate::AudioError;

pub(crate) fn volume_percent(direction: Direction) -> Result<Option<u32>, AudioError> {
    let _ = direction;
    Err(AudioError::Unsupported)
}

pub(crate) fn set_volume_percent(direction: Direction, percent: u32) -> Result<(), AudioError> {
    let _ = (direction, percent);
    Err(AudioError::Unsupported)
}
