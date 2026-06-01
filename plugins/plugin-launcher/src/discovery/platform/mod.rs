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

#[derive(Debug, Clone)]
pub struct AppRoot {
    pub path: PathBuf,
    pub max_depth: usize,
}

impl AppRoot {
    pub fn watch_recursive(&self) -> bool {
        self.max_depth <= 1
    }
}

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
