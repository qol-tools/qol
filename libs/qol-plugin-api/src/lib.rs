#[cfg(feature = "app-icons")]
pub mod app_icon;

pub mod color;
pub mod config;
pub mod daemon;
pub mod frecency;
pub mod platform;
pub mod search;

#[cfg(feature = "gpui")]
pub mod keepalive;
#[cfg(feature = "gpui")]
pub mod monitor;
#[cfg(feature = "gpui")]
pub mod window;

pub use qol_runtime::protocol;
pub use qol_runtime::{
    CursorPos, MonitorBounds, PlatformState, PlatformStateClient, Subscription, WindowBounds,
};
