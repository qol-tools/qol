use anyhow::{Context, Result};
use serialport::SerialPortInfo;

use super::{DoctorPlatformMetadata, SerialMetadata};

pub(crate) fn doctor_platform_metadata() -> DoctorPlatformMetadata {
    DoctorPlatformMetadata {
        name: "Linux",
        supported: true,
        serial_enumeration:
            "sysfs serial metadata with port opening and coordinator probing disabled",
    }
}

pub(crate) fn enumerate_serial_metadata() -> Result<SerialMetadata> {
    let ports =
        serialport::available_ports().context("failed to enumerate Linux serial metadata")?;
    Ok(SerialMetadata {
        source: "sysfs",
        ports,
    })
}

pub(crate) fn detect_coordinator_port(ports: &[SerialPortInfo]) -> Option<String> {
    super::port_detection::select_best_port(ports, score_port)
}

pub(crate) fn candidate_coordinator_ports(ports: &[SerialPortInfo]) -> Vec<String> {
    super::port_detection::ranked_port_names(ports, candidate_score)
}

fn score_port(port: &SerialPortInfo) -> Option<u16> {
    let mut score = super::port_detection::base_usb_score(port)?;
    let name = super::port_detection::port_name(port);

    if name.starts_with("/dev/serial/by-id/") {
        score = score.max(240);
    }
    if name.starts_with("/dev/ttyusb") || name.starts_with("/dev/ttyacm") {
        score = score.max(180);
    }

    Some(score)
}

fn candidate_score(port: &SerialPortInfo) -> Option<u16> {
    let name = super::port_detection::port_name(port);
    if let Some(score) = score_port(port) {
        return Some(score);
    }
    if let Some(mut score) = super::port_detection::secondary_usb_score(port) {
        if name.starts_with("/dev/serial/by-id/") {
            score = score.max(240);
        }
        if name.starts_with("/dev/ttyusb") || name.starts_with("/dev/ttyacm") {
            score = score.max(180);
        }
        return Some(score);
    }
    if name.starts_with("/dev/serial/by-id/") {
        return Some(170);
    }
    if name.starts_with("/dev/ttyusb") || name.starts_with("/dev/ttyacm") {
        return Some(150);
    }
    None
}
