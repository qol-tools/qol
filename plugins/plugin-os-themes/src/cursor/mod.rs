mod control;
mod platform;

use anyhow::Result;

use crate::config::Config;

pub(crate) use control::request_external_stop;
pub use control::{RunControl, RunState};
pub use platform::install_signal_handlers;

pub trait CursorEffect: Send + Sync {
    fn run(&self, config: &Config, control: &dyn RunControl) -> Result<()>;
}

pub fn create_effect() -> Box<dyn CursorEffect> {
    platform::create_effect()
}

pub fn open_settings() -> Result<()> {
    platform::open_settings()
}
