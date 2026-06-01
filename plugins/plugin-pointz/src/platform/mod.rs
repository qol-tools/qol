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

pub const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-pointz/";
