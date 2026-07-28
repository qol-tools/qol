use anyhow::{anyhow, Result};

use crate::config::Config;
use crate::cursor::{CursorEffect, RunControl};

use super::CursorPlatform;

pub struct Platform;

impl CursorPlatform for Platform {
    fn create_effect(&self) -> Box<dyn CursorEffect> {
        Box::new(UnsupportedEffect)
    }

    fn install_signal_handlers(&self) {}

    fn reset_external_stop(&self) {}

    fn external_stop_requested(&self) -> bool {
        false
    }
}

struct UnsupportedEffect;

impl CursorEffect for UnsupportedEffect {
    fn run(&self, _config: &Config, control: &dyn RunControl) -> Result<()> {
        let _ = control.should_stop();
        Err(anyhow!(
            "plugin-os-themes: cursor effects are not implemented on Windows"
        ))
    }
}
