pub(crate) mod control;
#[cfg(target_os = "linux")]
mod journal;
mod platform;

use anyhow::Result;

use crate::config::Config;

pub use control::{RunControl, RunState};
pub use platform::{CursorPlatform, Platform};

#[cfg(target_os = "linux")]
pub fn recover() {
    platform::recover_linux();
}

pub trait CursorEffect: Send + Sync {
    fn run(&self, config: &Config, control: &dyn RunControl) -> Result<()>;
}
