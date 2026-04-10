use anyhow::{Context, Result};
use serialport::SerialPortInfo;
use std::process::{Command, Stdio};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-lights/";

pub fn open_settings() -> Result<()> {
    Command::new("xdg-open")
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

    if name.starts_with("/dev/serial/by-id/") {
        score = score.max(240);
    }
    if name.starts_with("/dev/ttyusb") || name.starts_with("/dev/ttyacm") {
        score = score.max(180);
    }

    Some(score)
}

fn candidate_score(port: &SerialPortInfo) -> Option<u16> {
    let name = super::port_name(port);
    if let Some(score) = score_port(port) {
        return Some(score);
    }
    if let Some(mut score) = super::secondary_usb_score(port) {
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
