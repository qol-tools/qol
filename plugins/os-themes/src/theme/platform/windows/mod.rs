mod personalize;

use anyhow::Result;

use crate::session::{RestoreMode, RestoreReport};
use crate::theme::ColorScheme;

use super::ThemePlatform;

pub struct Platform;

impl ThemePlatform for Platform {
    fn current_scheme(&self) -> Result<ColorScheme> {
        personalize::current_scheme()
    }

    fn apply_scheme(&self, target: ColorScheme) -> Result<()> {
        personalize::apply_scheme(target)
    }

    fn restore(&self, _mode: RestoreMode, _report: &mut RestoreReport) {
        log::debug!("theme restore skipped: Windows keeps the chosen scheme as a user preference");
    }
}
