use crate::devices::Direction;
use crate::platform;
use crate::AudioError;

pub fn is_muted(direction: Direction) -> Result<Option<bool>, AudioError> {
    platform::is_muted(direction)
}

pub fn set_muted(direction: Direction, muted: bool) -> Result<(), AudioError> {
    platform::set_muted(direction, muted)
}
