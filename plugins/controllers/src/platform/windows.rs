use anyhow::{bail, Result};

use crate::detection::clash::LinkEvidence;
use crate::fixes::DetectedDevice;
use crate::platform::{NativeInputSnapshot, PlatformSupport};

pub(crate) fn platform_support() -> PlatformSupport {
    PlatformSupport {
        label: "Windows",
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

pub fn link_evidence(devices: &[DetectedDevice]) -> Vec<Option<LinkEvidence>> {
    vec![None; devices.len()]
}

pub fn disconnect_bluetooth(_device: &DetectedDevice) -> Result<String> {
    bail!("reconnecting a controller is only supported on Linux")
}
