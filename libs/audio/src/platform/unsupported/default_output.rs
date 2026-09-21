use crate::devices::{Direction, Identity};
use crate::AudioError;

pub(crate) fn effective_default(direction: Direction) -> Result<Option<String>, AudioError> {
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
