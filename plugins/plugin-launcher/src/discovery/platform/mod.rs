#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("platform implementation is required for this target OS");

use std::path::PathBuf;

use super::AppEntry;

pub fn cache_dir() -> Option<PathBuf> {
    imp::cache_dir()
}

pub fn app_watch_roots() -> Vec<PathBuf> {
    imp::app_watch_roots()
}

pub fn load_app_entries() -> Vec<AppEntry> {
    imp::load_app_entries()
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    imp::file_watch_roots()
}
