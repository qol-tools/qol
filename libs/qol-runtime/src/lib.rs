pub mod display;
pub mod keyremap_marker;
pub mod pane_field;
pub mod probe;
pub mod protocol;

pub mod broker;
pub mod local_ipc;

mod client;
pub mod plugin_config;
mod types;

mod watchdog;

pub use display::x11 as xrandr;
pub use pane_field::PaneField;

pub use client::{PlatformStateClient, Subscription};
pub use types::{CursorPos, MonitorBounds, PlatformState, WindowBounds};
pub use watchdog::{spawn_host_death_watchdog, spawn_host_death_watchdog_with};
