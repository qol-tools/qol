use anyhow::{anyhow, Result};

use crate::config::Config;
use crate::cursor::{CursorEffect, RunControl};

use super::CursorPlatform;

pub struct Platform;

impl CursorPlatform for Platform {
    fn create_effect(&self) -> Box<dyn CursorEffect> {
        Box::new(UnsupportedEffect)
    }

    fn install_signal_handlers(&self) {
        // No-op: nothing to wire up when the cursor effect refuses to run.
    }

    fn open_settings(&self) -> Result<()> {
        Err(anyhow!(
            "plugin-os-themes: settings UI is not implemented on macOS"
        ))
    }
}

struct UnsupportedEffect;

impl CursorEffect for UnsupportedEffect {
    fn run(&self, _config: &Config, _control: &dyn RunControl) -> Result<()> {
        Err(anyhow!(
            "plugin-os-themes: cursor effects are not implemented on macOS"
        ))
    }
}
