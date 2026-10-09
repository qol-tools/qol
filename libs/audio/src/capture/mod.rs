use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::platform;
use crate::AudioError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pcm16Format {
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct Capture {
    inner: platform::Capture,
    stop: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct CaptureStop(Arc<AtomicBool>);

impl Capture {
    pub fn open(device: Option<&str>, format: Pcm16Format) -> Result<Self, AudioError> {
        let stop = Arc::new(AtomicBool::new(false));
        let inner = platform::Capture::open(device, format, Arc::clone(&stop))?;
        Ok(Self { inner, stop })
    }

    pub fn device(&self) -> &str {
        self.inner.device()
    }

    pub fn stopper(&self) -> CaptureStop {
        CaptureStop(Arc::clone(&self.stop))
    }
}

impl Read for Capture {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl CaptureStop {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Release);
    }
}
