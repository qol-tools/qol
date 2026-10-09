use std::path::PathBuf;

use crate::AppRoot;

trait StartMenuPlatform {
    fn program_roots(&self) -> Vec<AppRoot>;
    fn shortcut_paths(&self, root: &AppRoot) -> Vec<PathBuf>;
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

pub(super) fn program_roots() -> Vec<AppRoot> {
    imp::Platform.program_roots()
}

pub(super) fn shortcut_paths(root: &AppRoot) -> Vec<PathBuf> {
    imp::Platform.shortcut_paths(root)
}
