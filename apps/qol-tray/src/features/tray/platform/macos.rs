use crate::features::plugin_manager::PluginManager;
use anyhow::Result;
use std::sync::Arc;
use std::sync::Mutex;
use tray_icon::{TrayIconBuilder, TrayIcon, Icon};

pub fn create_tray(plugin_manager: Arc<Mutex<PluginManager>>, icon: Icon) -> Result<TrayIcon> {
    let menu = crate::tray::menu::build_menu(&plugin_manager);

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("QoL Tray")
        .with_icon(icon)
        .build()?;

    Ok(tray_icon)
}
