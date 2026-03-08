#[cfg(feature = "app-icons")]
pub mod app_icon;

#[cfg(unix)]
pub mod daemon;

#[cfg(feature = "gpui")]
pub mod keepalive;
#[cfg(feature = "gpui")]
pub mod monitor;
#[cfg(feature = "gpui")]
pub mod window;

pub use qol_runtime::protocol;
pub use qol_runtime::{CursorPos, MonitorBounds, PlatformState, WindowBounds};
#[cfg(unix)]
pub use qol_runtime::{PlatformStateClient, Subscription};
