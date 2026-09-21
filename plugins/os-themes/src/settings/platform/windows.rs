use anyhow::{anyhow, Result};

use super::SettingsPlatform;
use crate::config::PLUGIN_ID;

pub(in crate::settings) struct Platform;

impl SettingsPlatform for Platform {
    fn open(&self) -> Result<()> {
        Err(anyhow!(
            "{PLUGIN_ID}: settings UI is not implemented on Windows"
        ))
    }
}
