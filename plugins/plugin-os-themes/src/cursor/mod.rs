pub mod platform;

use anyhow::Result;

pub use platform::open_settings;
pub use platform::request_reload;
pub use platform::request_shutdown;
pub use platform::reset_running;
pub use platform::run;
pub use platform::was_reload_requested;

pub trait CursorEffect {
    fn start(&self) -> Result<()>;
    fn stop(&self) -> Result<()>;
}
