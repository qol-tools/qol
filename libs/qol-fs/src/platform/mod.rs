#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{set_private, set_private_dir, sync_parent};
#[cfg(target_os = "macos")]
pub(crate) use macos::{set_private, set_private_dir, sync_parent};
#[cfg(target_os = "windows")]
pub(crate) use windows::{set_private, set_private_dir, sync_parent};
