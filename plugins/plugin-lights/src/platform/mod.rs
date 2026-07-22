use anyhow::{Context, Result};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod port_description;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod port_detection;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) use fallback::{candidate_coordinator_ports, detect_coordinator_port};
#[cfg(target_os = "linux")]
pub(crate) use linux::{candidate_coordinator_ports, detect_coordinator_port};
#[cfg(target_os = "macos")]
pub(crate) use macos::{candidate_coordinator_ports, detect_coordinator_port};
pub(crate) use port_description::describe_port;

use crate::config::store::PLUGIN_ID;

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
        .context("failed to open settings URL")
}
