use anyhow::{bail, Result};
use serialport::SerialPortInfo;

use super::{DoctorPlatformMetadata, SerialMetadata};

pub(crate) fn doctor_platform_metadata() -> DoctorPlatformMetadata {
    DoctorPlatformMetadata {
        name: "Windows",
        supported: false,
        serial_enumeration: "disabled because Lights is not declared for Windows",
    }
}

pub(crate) fn enumerate_serial_metadata() -> Result<SerialMetadata> {
    bail!("serial metadata enumeration is unavailable because Lights does not support Windows")
}

pub(crate) fn detect_coordinator_port(_ports: &[SerialPortInfo]) -> Option<String> {
    None
}

pub(crate) fn candidate_coordinator_ports(_ports: &[SerialPortInfo]) -> Vec<String> {
    Vec::new()
}
