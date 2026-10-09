use crate::devices::Direction;
use crate::AudioError;

pub(crate) enum Meter {}

impl Meter {
    pub(crate) fn open(direction: Direction) -> Result<Self, AudioError> {
        let _ = direction;
        Err(AudioError::Unsupported)
    }

    pub(crate) fn peak(&self) -> f32 {
        match *self {}
    }

    pub(crate) fn device(&self) -> &str {
        match *self {}
    }

    pub(crate) fn peak_before_volume(&self, percent: u32) -> f32 {
        let _ = percent;
        match *self {}
    }
}
