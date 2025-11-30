#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

use crate::plugins::PluginManager;
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

#[cfg(not(target_os = "linux"))]
pub fn create_tray(plugin_manager: Arc<Mutex<PluginManager>>, icon: Icon) -> Result<PlatformTray> {
    let (menu, router) = crate::menu::builder::build_menu(plugin_manager)?;

    #[cfg(target_os = "windows")]
    let tray = windows::create_tray(menu, router, icon)?;

    #[cfg(target_os = "macos")]
    let tray = macos::create_tray(menu, router, icon)?;

    Ok(PlatformTray::Standard(tray))
}
