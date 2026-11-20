use anyhow::{Context, Result};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open settings URL")
}
