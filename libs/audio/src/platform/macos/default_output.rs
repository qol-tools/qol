use crate::devices::{Direction, Identity};
use crate::AudioError;

use super::hal::{self, ObjectId, Property};
use super::identity;

pub(super) fn default_device(direction: Direction) -> Result<Option<ObjectId>, AudioError> {
    let id = hal::read_u32(hal::SYSTEM, Property::default_device(direction))?;
    Ok((id != hal::NO_OBJECT).then_some(id))
}

pub(crate) fn effective_default(direction: Direction) -> Result<Option<String>, AudioError> {
    default_device(direction)?
        .map(|id| hal::read_text(id, Property::global(hal::DEVICE_UID)))
        .transpose()
}

pub(crate) fn set_default_output(
    direction: Direction,
    output: &Identity,
) -> Result<(), AudioError> {
    let id = identity::device_for(direction, output)?
        .ok_or_else(|| AudioError::not_present(direction, output))?;
    hal::write_u32(hal::SYSTEM, Property::default_device(direction), id)
}
