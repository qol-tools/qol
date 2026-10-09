use anyhow::{anyhow, Result};

use crate::session::{RestoreMode, RestoreReport};
use crate::theme::ColorScheme;

use super::ThemePlatform;

pub struct Platform;

impl ThemePlatform for Platform {
    fn current_scheme(&self) -> Result<ColorScheme> {
        Err(anyhow!("theme switching is not implemented on Windows"))
    }

    fn apply_scheme(&self, _target: ColorScheme) -> Result<()> {
        Err(anyhow!("theme switching is not implemented on Windows"))
    }

    fn restore(&self, _mode: RestoreMode, _report: &mut RestoreReport) {}
}
