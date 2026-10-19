#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{file_id, is_hidden, is_hidden_item, mounts, system_root, user_folders};
#[cfg(target_os = "linux")]
pub(crate) use linux::{file_id, is_hidden, is_hidden_item, mounts, system_root, user_folders};
#[cfg(target_os = "macos")]
pub(crate) use macos::{file_id, is_hidden, is_hidden_item, mounts, system_root, user_folders};
#[cfg(target_os = "windows")]
pub(crate) use windows::{file_id, is_hidden, is_hidden_item, mounts, system_root, user_folders};
