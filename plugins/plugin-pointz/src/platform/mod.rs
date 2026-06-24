//! Platform abstraction for OS-specific shell interactions.
//!
//! Uses the re-export shape (`pub use <os>::*`) — each `<os>.rs` exports the
//! same set of public symbols. See `workspace/.claude/skills/qol-architecture`
//! for the design rules.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod other;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use other::*;
#[cfg(target_os = "windows")]
pub use windows::*;

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn settings_url() -> String {
    qol_conventions::settings_url(PLUGIN_ID)
}
