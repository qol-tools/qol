pub(crate) mod control;
mod platform;

use anyhow::Result;

use crate::config::Config;

pub use control::{RunControl, RunState};
pub use platform::{CursorPlatform, Platform};

pub trait CursorEffect: Send + Sync {
    fn run(&self, config: &Config, control: &dyn RunControl) -> Result<()>;
}
