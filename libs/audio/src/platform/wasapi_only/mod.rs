use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::capture::{Pcm16Format, Source};
use crate::AudioError;

pub(crate) enum Capture {}

impl Capture {
    pub(crate) fn open(
        source: Source,
        device: Option<&str>,
        format: Pcm16Format,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, AudioError> {
        let _ = (source, device, format, stop);
        Err(AudioError::Unsupported)
    }

    pub(crate) fn device(&self) -> &str {
        match *self {}
    }

    pub(crate) fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let _ = buffer;
        match *self {}
    }
}
