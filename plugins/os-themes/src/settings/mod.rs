use anyhow::{Context, Result};

pub(crate) fn open() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(crate::config::PLUGIN_ID)
        .context("failed to open settings URL")
}
