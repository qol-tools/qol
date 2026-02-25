pub mod color;
pub mod config;
pub mod daemon;
pub mod frecency;
pub mod keepalive;
pub mod monitor;
pub mod platform;
pub mod search;
pub mod window;

pub use qol_runtime::{CursorPos, MonitorBounds, PlatformState, PlatformStateClient, WindowBounds};
