use anyhow::{anyhow, Result};

use crate::theme::ColorScheme;

use super::DesktopBackend;

pub(in super::super) struct Kde;

impl DesktopBackend for Kde {
    fn current_scheme(&self) -> Result<ColorScheme> {
        Err(anyhow!("KDE theme switching is not implemented yet"))
    }

    fn apply(&self, _target: ColorScheme) -> Result<()> {
        Err(anyhow!("KDE theme switching is not implemented yet"))
    }
}
