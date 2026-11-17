#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{pre_create_selector, select_region, select_region_in_app};
#[cfg(target_os = "macos")]
pub(crate) use macos::{pre_create_selector, select_region, select_region_in_app};
#[cfg(target_os = "windows")]
pub(crate) use windows::{pre_create_selector, select_region, select_region_in_app};
