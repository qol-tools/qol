use crate::devices::Direction;
use crate::platform;
use crate::AudioError;

pub struct Meter {
    inner: platform::Meter,
}

impl Meter {
    pub fn open(direction: Direction) -> Result<Self, AudioError> {
        Ok(Self {
            inner: platform::Meter::open(direction)?,
        })
    }

    pub fn peak(&self) -> f32 {
        self.inner.peak()
    }

    pub fn device(&self) -> &str {
        self.inner.device()
    }

    pub fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    pub fn peak_before_volume(&self, percent: u32) -> f32 {
        self.inner.peak_before_volume(percent)
    }
}
