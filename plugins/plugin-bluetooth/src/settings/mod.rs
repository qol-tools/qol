use anyhow::{Context, Result};

mod platform;

pub(crate) fn run_panel() -> Result<()> {
    platform::run_panel()
}

pub(crate) fn open_browser() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(crate::PLUGIN_ID)
        .context("failed to open Bluetooth settings URL")
}
