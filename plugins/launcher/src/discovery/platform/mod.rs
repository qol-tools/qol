#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

use std::path::PathBuf;

use super::AppEntry;
pub use qol_apps::AppRoot;

pub fn cache_dir() -> Option<PathBuf> {
    imp::cache_dir()
}

pub fn app_roots() -> Vec<AppRoot> {
    imp::app_roots()
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    imp::scan_root(root)
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    imp::file_watch_roots()
}
