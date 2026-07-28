use crate::fixes::DetectedDevice;
use crate::platform::{NativeInputSnapshot, PlatformSupport};

pub(crate) fn platform_support() -> PlatformSupport {
    PlatformSupport {
        label: "macOS",
        supported: false,
    }
}

#[derive(Default)]
pub struct InputMonitor;

impl InputMonitor {
    pub fn snapshot(&mut self) -> NativeInputSnapshot {
        NativeInputSnapshot {
            available: false,
            source: None,
            items: Vec::new(),
        }
    }
}

pub fn read_devices() -> Vec<DetectedDevice> {
    Vec::new()
}
