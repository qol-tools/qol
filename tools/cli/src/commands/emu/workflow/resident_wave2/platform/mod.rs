#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{file_mode, rename_noreplace};
#[cfg(target_os = "macos")]
pub(crate) use macos::{file_mode, rename_noreplace};
#[cfg(target_os = "windows")]
pub(crate) use windows::{file_mode, rename_noreplace};
