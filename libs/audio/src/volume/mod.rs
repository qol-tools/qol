use crate::devices::Direction;
use crate::platform;
use crate::AudioError;

pub fn output_percent() -> Result<Option<u32>, AudioError> {
    platform::volume_percent(Direction::Output)
}

pub fn set_output_percent(percent: u32) -> Result<(), AudioError> {
    platform::set_volume_percent(Direction::Output, percent)
}

pub fn input_percent() -> Result<Option<u32>, AudioError> {
    platform::volume_percent(Direction::Input)
}

pub fn set_input_percent(percent: u32) -> Result<(), AudioError> {
    platform::set_volume_percent(Direction::Input, percent)
}
