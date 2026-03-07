pub mod platform;

use anyhow::Result;

pub use platform::open_settings;
pub use platform::run;

pub trait CursorEffect {
    fn start(&self) -> Result<()>;
    fn stop(&self) -> Result<()>;
}
