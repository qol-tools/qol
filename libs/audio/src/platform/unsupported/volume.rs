use crate::AudioError;

pub(crate) fn output_volume_percent() -> Result<Option<u32>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn set_output_volume_percent(percent: u32) -> Result<(), AudioError> {
    let _ = percent;
    Err(AudioError::Unsupported)
}
