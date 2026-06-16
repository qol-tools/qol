pub mod keyremap_marker;
pub mod pane_field;
pub mod probe;
pub mod protocol;
pub mod xrandr;

#[cfg(unix)]
pub mod broker;

#[cfg(unix)]
mod client;
mod types;

#[cfg(unix)]
mod watchdog;

pub use pane_field::PaneField;

#[cfg(unix)]
pub use client::{PlatformStateClient, Subscription};
pub use types::{CursorPos, MonitorBounds, PlatformState, WindowBounds};
#[cfg(unix)]
pub use watchdog::spawn_host_death_watchdog;
