use anyhow::{Context, Result};
use serialport::SerialPortInfo;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod port_description;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod port_detection;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    candidate_coordinator_ports, detect_coordinator_port, doctor_platform_metadata,
    enumerate_serial_metadata,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    candidate_coordinator_ports, detect_coordinator_port, doctor_platform_metadata,
    enumerate_serial_metadata,
};
pub(crate) use port_description::describe_port;
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    candidate_coordinator_ports, detect_coordinator_port, doctor_platform_metadata,
    enumerate_serial_metadata,
};

use crate::config::store::PLUGIN_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DoctorPlatformMetadata {
    pub name: &'static str,
    pub supported: bool,
    pub serial_enumeration: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct SerialMetadata {
    pub source: &'static str,
    pub ports: Vec<SerialPortInfo>,
}

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
        .context("failed to open settings URL")
}
