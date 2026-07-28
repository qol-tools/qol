use anyhow::{Context, Result};

use super::SettingsPlatform;

pub(in crate::settings) struct Platform;

impl SettingsPlatform for Platform {
    fn open(&self) -> Result<()> {
        qol_apps::desktop_integration::open_plugin_settings(crate::config::PLUGIN_ID)
            .context("failed to open settings URL")
    }
}
