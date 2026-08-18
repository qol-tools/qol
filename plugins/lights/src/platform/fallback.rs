use anyhow::{bail, Result};
use serialport::SerialPortInfo;

use super::{DoctorPlatformMetadata, SerialAccess, SerialMetadata};

pub(crate) fn doctor_platform_metadata() -> DoctorPlatformMetadata {
    DoctorPlatformMetadata {
        name: std::env::consts::OS,
        supported: false,
        serial_enumeration: "disabled because Lights is not declared for this platform",
    }
}

pub(crate) fn enumerate_serial_metadata() -> Result<SerialMetadata> {
    bail!(
        "serial metadata enumeration is unavailable because Lights does not support this platform"
    )
}

pub(crate) fn detect_coordinator_port(_ports: &[SerialPortInfo]) -> Option<String> {
    None
}

pub(crate) fn candidate_coordinator_ports(_ports: &[SerialPortInfo]) -> Vec<String> {
    Vec::new()
}

pub(crate) fn inspect_serial_access(path: &str) -> SerialAccess {
    SerialAccess {
        path: path.to_string(),
        readable_writable: false,
        issue: Some(
            "serial access inspection is unavailable on this unsupported platform".to_string(),
        ),
    }
}
