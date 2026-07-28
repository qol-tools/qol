use anyhow::{anyhow, Result};

use super::SettingsPlatform;

pub(in crate::settings) struct Platform;

impl SettingsPlatform for Platform {
    fn open(&self) -> Result<()> {
        Err(anyhow!(
            "plugin-os-themes: settings UI is not implemented on Windows"
        ))
    }
}
