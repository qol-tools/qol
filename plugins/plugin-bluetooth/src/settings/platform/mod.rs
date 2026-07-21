#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{run_panel, spawn_panel};
#[cfg(target_os = "macos")]
pub(super) use macos::{run_panel, spawn_panel};
#[cfg(target_os = "windows")]
pub(super) use windows::{run_panel, spawn_panel};
