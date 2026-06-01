use anyhow::{Context, Result};
use serialport::SerialPortInfo;
use std::process::{Command, Stdio};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-lights/";

pub fn open_settings() -> Result<()> {
    Command::new("open")
        .arg(SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}

pub fn detect_coordinator_port(ports: &[SerialPortInfo]) -> Option<String> {
    super::select_best_port(ports, score_port)
}

pub fn candidate_coordinator_ports(ports: &[SerialPortInfo]) -> Vec<String> {
    super::ranked_port_names(ports, candidate_score)
}

fn score_port(port: &SerialPortInfo) -> Option<u16> {
    let mut score = super::base_usb_score(port)?;
    let name = super::port_name(port);

    if name.starts_with("/dev/cu.usbmodem") || name.starts_with("/dev/cu.usbserial") {
        score = score.max(160);
    }
    if name.starts_with("/dev/tty.usbmodem") || name.starts_with("/dev/tty.usbserial") {
        score = score.max(140);
    }

    Some(score)
}

fn candidate_score(port: &SerialPortInfo) -> Option<u16> {
    let name = super::port_name(port);
    if let Some(mut score) = score_port(port) {
        if name.starts_with("/dev/cu.") {
            score = score.max(180);
        }
        if name.starts_with("/dev/tty.") {
            score = score.max(160);
        }
        return Some(score);
    }

    if let Some(mut score) = super::secondary_usb_score(port) {
        if name.starts_with("/dev/cu.") {
            score = score.max(180);
        }
        if name.starts_with("/dev/tty.") {
            score = score.max(160);
        }
        return Some(score);
    }

    if name.starts_with("/dev/cu.usbmodem") || name.starts_with("/dev/cu.usbserial") {
        return Some(150);
    }
    if name.starts_with("/dev/tty.usbmodem") || name.starts_with("/dev/tty.usbserial") {
        return Some(130);
    }
    if name.starts_with("/dev/cu.slab_usbtouart") || name.starts_with("/dev/cu.wchusbserial") {
        return Some(150);
    }
    if name.starts_with("/dev/tty.slab_usbtouart") || name.starts_with("/dev/tty.wchusbserial") {
        return Some(130);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::{SerialPortInfo, SerialPortType, UsbPortInfo};

    #[test]
    fn detect_prefers_identified_zigbee_dongle() {
        let ports = vec![
            usb_port(
                "/dev/cu.usbmodem000000015",
                0x10C4,
                0xEA60,
                Some("Silicon Labs"),
                Some("Sonoff Zigbee 3.0 USB Dongle Plus"),
            ),
            unknown_port("/dev/cu.Bluetooth-Incoming-Port"),
        ];
        let port = detect_coordinator_port(&ports);
        assert_eq!(port.as_deref(), Some("/dev/cu.usbmodem000000015"));
    }

    #[test]
    fn detect_rejects_generic_usbmodem_without_zigbee_identity() {
        let ports = vec![
            unknown_port("/dev/cu.usbmodem000000015"),
            unknown_port("/dev/cu.Bluetooth-Incoming-Port"),
        ];
        let port = detect_coordinator_port(&ports);
        assert!(port.is_none());
    }

    #[test]
    fn detect_rejects_non_zigbee_usb_audio_device() {
        let ports = vec![usb_port(
            "/dev/cu.usbmodem000000015",
            0x03F0,
            0x098D,
            Some("HP, Inc"),
            Some("HyperX Cloud Alpha Wireless"),
        )];
        let port = detect_coordinator_port(&ports);
        assert!(port.is_none());
    }

    #[test]
    fn detect_does_not_auto_select_soft_usb_identity() {
        let ports = vec![usb_port(
            "/dev/cu.usbserial-0001",
            0x10C4,
            0xEA70,
            Some("Silicon Labs"),
            Some("CP2102 USB to UART Bridge Controller"),
        )];
        let port = detect_coordinator_port(&ports);
        assert!(port.is_none());
    }

    #[test]
    fn candidates_include_soft_identity_and_generic_usb_serial_ports() {
        let ports = vec![
            usb_port(
                "/dev/cu.usbserial-0001",
                0x10C4,
                0xEA70,
                Some("Silicon Labs"),
                Some("CP2102 USB to UART Bridge Controller"),
            ),
            unknown_port("/dev/cu.usbmodem000000015"),
            unknown_port("/dev/cu.Bluetooth-Incoming-Port"),
        ];
        let ports = candidate_coordinator_ports(&ports);
        assert_eq!(
            ports,
            vec![
                "/dev/cu.usbserial-0001".to_string(),
                "/dev/cu.usbmodem000000015".to_string(),
            ]
        );
    }

    fn unknown_port(name: &str) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.to_string(),
            port_type: SerialPortType::Unknown,
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
