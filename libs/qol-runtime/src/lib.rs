#[cfg(unix)]
mod client;
mod types;

#[cfg(unix)]
pub use client::PlatformStateClient;
pub use types::{CursorPos, MonitorBounds, PlatformState, WindowBounds};
