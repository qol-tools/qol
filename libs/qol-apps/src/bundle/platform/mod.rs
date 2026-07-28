use std::path::{Path, PathBuf};

use crate::AppRoot;

trait BundlePlatform {
    fn cache_dir(&self) -> Option<PathBuf>;
    fn launcher_roots(&self) -> Vec<AppRoot>;
    fn bundle_info(&self, path: &Path) -> (Option<String>, Option<String>);
    fn spotlight_app_paths(&self, roots: &[PathBuf]) -> Vec<PathBuf>;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
mod launcher;
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

pub(super) fn launcher_roots() -> Vec<AppRoot> {
    imp::Platform.launcher_roots()
}

pub(super) fn launcher_exec(path: &Path) -> Vec<String> {
    launcher::exec(path)
}

pub(super) fn bundle_info(path: &Path) -> (Option<String>, Option<String>) {
    imp::Platform.bundle_info(path)
}

pub(super) fn spotlight_app_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    imp::Platform.spotlight_app_paths(roots)
}
