mod platform;
mod icon;

use crate::plugins::PluginManager;
use anyhow::Result;
use std::sync::{Arc, Mutex};

pub struct TrayManager {
    _tray: platform::PlatformTray,
}

impl TrayManager {
    pub fn new(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<Self> {
        let icon = icon::create_icon();
        let tray = platform::create_tray(plugin_manager, icon)?;
        Ok(Self { _tray: tray })
    }
}
