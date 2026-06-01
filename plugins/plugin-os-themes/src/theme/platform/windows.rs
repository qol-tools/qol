use anyhow::{anyhow, Result};

use super::ThemePlatform;

pub struct Platform;

impl ThemePlatform for Platform {
    fn apply_theme(&self, _name: &str) -> Result<()> {
        Err(anyhow!(
            "plugin-os-themes: OS-wide theming is not implemented on Windows"
        ))
    }
}
