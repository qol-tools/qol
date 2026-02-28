pub mod protocol;

#[cfg(unix)]
mod client;
mod types;

#[cfg(unix)]
pub use client::{PlatformStateClient, Subscription};
pub use types::{CursorPos, MonitorBounds, PlatformState, WindowBounds};
