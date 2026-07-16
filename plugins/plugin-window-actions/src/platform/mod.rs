#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{execute_action, state_file_path, GlideController};
#[cfg(target_os = "macos")]
pub(crate) use macos::{execute_action, state_file_path, GlideController};
#[cfg(target_os = "windows")]
pub(crate) use windows::{execute_action, state_file_path, GlideController};
