use anyhow::{anyhow, Result};

use crate::theme::ColorScheme;

use super::ThemePlatform;

pub struct Platform;

impl ThemePlatform for Platform {
    fn current_scheme(&self) -> Result<ColorScheme> {
        Err(anyhow!(
            "plugin-os-themes: theme switching is not implemented on this platform"
        ))
    }

    fn apply_scheme(&self, _target: ColorScheme) -> Result<()> {
        Err(anyhow!(
            "plugin-os-themes: theme switching is not implemented on this platform"
        ))
    }
}
