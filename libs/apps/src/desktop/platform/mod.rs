use std::path::PathBuf;

use crate::AppRoot;

trait DesktopPlatform {
    fn cache_dir(&self) -> Option<PathBuf>;
    fn app_roots(&self) -> Vec<AppRoot>;
}

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

pub(super) fn cache_dir() -> Option<PathBuf> {
    imp::Platform.cache_dir()
}

pub(super) fn app_roots() -> Vec<AppRoot> {
    imp::Platform.app_roots()
}
