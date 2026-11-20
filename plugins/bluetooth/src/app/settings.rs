use anyhow::{Context, Result};

pub fn show_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(crate::PLUGIN_ID)
        .context("failed to open Bluetooth settings URL")
}
