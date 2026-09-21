use crate::platform;
use crate::AudioError;

pub fn output_percent() -> Result<Option<u32>, AudioError> {
    platform::output_volume_percent()
}

pub fn set_output_percent(percent: u32) -> Result<(), AudioError> {
    platform::set_output_volume_percent(percent)
}
