use anyhow::{Context, Result};

pub(crate) fn open_browser() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(crate::PLUGIN_ID)
        .context("failed to open Bluetooth settings URL")
}
