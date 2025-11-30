#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

use crate::features::plugin_manager::PluginManager;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tray_icon::Icon;

#[cfg(not(target_os = "linux"))]
use tray_icon::TrayIcon;

pub enum PlatformTray {
    #[cfg(target_os = "linux")]
    Linux,
    #[cfg(not(target_os = "linux"))]
    Standard(TrayIcon),
}

#[cfg(target_os = "linux")]
pub fn create_tray(plugin_manager: Arc<Mutex<PluginManager>>, icon: Icon) -> Result<PlatformTray> {
    linux::create_tray(plugin_manager, icon)?;
    Ok(PlatformTray::Linux)
}

#[cfg(target_os = "windows")]
pub fn create_tray(plugin_manager: Arc<Mutex<PluginManager>>, icon: Icon) -> Result<PlatformTray> {
    let tray = windows::create_tray(plugin_manager, icon)?;
    Ok(PlatformTray::Standard(tray))
}

#[cfg(target_os = "macos")]
pub fn create_tray(plugin_manager: Arc<Mutex<PluginManager>>, icon: Icon) -> Result<PlatformTray> {
    let tray = macos::create_tray(plugin_manager, icon)?;
    Ok(PlatformTray::Standard(tray))
}
