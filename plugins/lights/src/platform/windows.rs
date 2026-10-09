use anyhow::{Context, Result};
use serialport::{SerialPortInfo, SerialPortType};

use super::{DoctorPlatformMetadata, SerialAccess, SerialMetadata};

const DEVICE_NAMESPACE: &str = r"\\.\";
const GENERIC_USB_SERIAL_SCORE: u16 = 150;

pub(crate) fn doctor_platform_metadata() -> DoctorPlatformMetadata {
    DoctorPlatformMetadata {
        name: "Windows",
        supported: true,
        serial_enumeration:
            "SetupAPI COM port metadata with port opening and coordinator probing disabled",
    }
}

pub(crate) fn enumerate_serial_metadata() -> Result<SerialMetadata> {
    let ports = serialport::available_ports().context("failed to enumerate Windows COM ports")?;
    Ok(SerialMetadata {
        source: "setupapi",
        ports,
    })
}

pub(crate) fn detect_coordinator_port(ports: &[SerialPortInfo]) -> Option<String> {
    super::port_detection::select_best_port(ports, score_port)
}

pub(crate) fn candidate_coordinator_ports(ports: &[SerialPortInfo]) -> Vec<String> {
    super::port_detection::ranked_port_names(ports, candidate_score)
}

pub(crate) fn serial_port_present(path: &str) -> bool {
    serialport::available_ports().is_ok_and(|ports| contains_port(&ports, path))
}

pub(crate) fn inspect_serial_access(path: &str) -> SerialAccess {
    let issue = match serialport::available_ports() {
        Ok(ports) if contains_port(&ports, path) => None,
        Ok(_) => Some(format!("{} is not a present COM port", com_name(path))),
        Err(error) => Some(format!("COM ports cannot be enumerated: {error}")),
    };
    SerialAccess {
        path: path.to_string(),
        readable_writable: issue.is_none(),
        issue,
    }
}

fn score_port(port: &SerialPortInfo) -> Option<u16> {
    super::port_detection::base_usb_score(port)
}

fn candidate_score(port: &SerialPortInfo) -> Option<u16> {
    score_port(port)
        .or_else(|| super::port_detection::secondary_usb_score(port))
        .or_else(|| {
            matches!(port.port_type, SerialPortType::UsbPort(_)).then_some(GENERIC_USB_SERIAL_SCORE)
        })
}

fn contains_port(ports: &[SerialPortInfo], path: &str) -> bool {
    let wanted = com_name(path);
    ports
        .iter()
        .any(|port| com_name(&port.port_name).eq_ignore_ascii_case(wanted))
}

fn com_name(path: &str) -> &str {
    let trimmed = path.trim();
    trimmed.strip_prefix(DEVICE_NAMESPACE).unwrap_or(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::UsbPortInfo;

    #[test]
    fn coordinator_detection_ranks_com_ports() {
        let sonoff = usb_port(
            "COM4",
            0x10C4,
            0xEA60,
            Some("Silicon Labs"),
            Some("Silicon Labs CP210x USB to UART Bridge (COM4)"),
        );
        let cp2102 = usb_port(
            "COM5",
            0x10C4,
            0xEA70,
            Some("Silicon Labs"),
            Some("Silicon Labs Dual CP2105 USB to UART Bridge (COM5)"),
        );
        let ch9102 = usb_port(
            "COM6",
            0x1A86,
            0x55D4,
            Some("wch.cn"),
            Some("USB-Enhanced-SERIAL CH9102 (COM6)"),
        );
        let legacy = SerialPortInfo {
            port_name: "COM1".to_string(),
            port_type: SerialPortType::Unknown,
        };
        let cases: [(Vec<SerialPortInfo>, Option<&str>, &[&str]); 4] = [
            (
                vec![legacy.clone(), sonoff.clone(), ch9102.clone()],
                Some("COM4"),
                &["COM4", "COM6"],
            ),
            (vec![legacy.clone(), cp2102.clone()], None, &["COM5"]),
            (vec![ch9102.clone()], None, &["COM6"]),
            (vec![legacy], None, &[]),
        ];
        for (ports, detected, candidates) in cases {
            assert_eq!(detect_coordinator_port(&ports).as_deref(), detected);
            assert_eq!(
                candidate_coordinator_ports(&ports),
                candidates
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn configured_com_names_match_enumerated_ports() {
        let ports = vec![usb_port("COM12", 0x10C4, 0xEA60, None, None)];
        let cases = [
            ("COM12", true),
            ("com12", true),
            (r"\\.\COM12", true),
            (" COM12 ", true),
            ("COM1", false),
            ("auto", false),
        ];
        for (path, expected) in cases {
            assert_eq!(contains_port(&ports, path), expected, "path={path:?}");
        }
    }

    fn usb_port(
        name: &str,
        vid: u16,
        pid: u16,
        manufacturer: Option<&str>,
        product: Option<&str>,
    ) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.to_string(),
            port_type: SerialPortType::UsbPort(UsbPortInfo {
                vid,
                pid,
                serial_number: None,
                manufacturer: manufacturer.map(str::to_string),
                product: product.map(str::to_string),
            }),
        }
    }
}
