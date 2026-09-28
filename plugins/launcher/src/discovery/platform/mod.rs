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

use std::path::{Path, PathBuf};

use super::details::{AppAbout, AppFace};
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

pub fn app_face(entry: &AppEntry) -> AppFace {
    imp::app_face(entry)
}

pub fn app_about(entry: &AppEntry) -> AppAbout {
    imp::app_about(entry)
}

pub fn file_icon(path: &Path) -> Option<PathBuf> {
    imp::file_icon(path)
}

pub fn recent_files(limit: usize) -> Vec<PathBuf> {
    imp::recent_files(limit)
}
