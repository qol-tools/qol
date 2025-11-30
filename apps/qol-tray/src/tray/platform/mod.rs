#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

use crate::plugins::PluginManager;
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
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
pub fn create_tray(
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
) -> Result<PlatformTray> {
    linux::create_tray(plugin_manager, feature_registry, shutdown_tx, icon)?;
    Ok(PlatformTray::Linux)
}

#[cfg(target_os = "linux")]
pub fn request_plugin_refresh() {
    linux::request_plugin_refresh();
}

#[cfg(not(target_os = "linux"))]
pub fn request_plugin_refresh() {
    log::warn!("Plugin refresh not yet implemented on this platform");
}

#[cfg(not(target_os = "linux"))]
pub fn create_tray(
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
) -> Result<PlatformTray> {
    let (menu, router) = crate::menu::builder::build_menu(plugin_manager, feature_registry)?;

    #[cfg(target_os = "windows")]
    let tray = windows::create_tray(menu, router, shutdown_tx, icon)?;

    #[cfg(target_os = "macos")]
    let tray = macos::create_tray(menu, router, shutdown_tx, icon)?;

    Ok(PlatformTray::Standard(tray))
}
