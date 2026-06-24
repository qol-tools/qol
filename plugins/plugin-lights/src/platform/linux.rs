use anyhow::{Context, Result};
use serialport::SerialPortInfo;
use std::process::{Command, Stdio};

use crate::config::store::PLUGIN_ID;

pub fn open_settings() -> Result<()> {
    let settings_url = qol_conventions::settings_url(PLUGIN_ID);
    Command::new("xdg-open")
        .arg(&settings_url)
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
