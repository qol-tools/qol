use super::state::SystemPaths;
use anyhow::Result;
use std::path::{Path, PathBuf};

mod fallback;
const _: fallback::Platform = fallback::Platform;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::Platform as imp;
#[cfg(target_os = "linux")]
use linux::Platform as imp;
#[cfg(target_os = "macos")]
use macos::Platform as imp;
#[cfg(target_os = "windows")]
use windows::Platform as imp;

trait FixPlatform {
    fn system_paths() -> SystemPaths;
    fn live_quirk_path(driver: &str) -> Option<String>;
    fn authorization_available() -> bool;
    fn apply(conf: &str, writes: &[(String, String)]) -> Result<()>;
    fn hidraw_guard_path() -> Option<PathBuf>;
    fn install_hidraw_guard(path: &Path, content: &str) -> Result<()>;
}

pub(super) fn system_paths() -> SystemPaths {
    imp::system_paths()
}

pub(super) fn live_quirk_path(driver: &str) -> Option<String> {
    imp::live_quirk_path(driver)
}

pub(crate) fn authorization_available() -> bool {
    imp::authorization_available()
}

pub(super) fn apply(conf: &str, writes: &[(String, String)]) -> Result<()> {
    imp::apply(conf, writes)
}

pub(super) fn hidraw_guard_path() -> Option<PathBuf> {
    imp::hidraw_guard_path()
}

pub(super) fn install_hidraw_guard(path: &Path, content: &str) -> Result<()> {
    imp::install_hidraw_guard(path, content)
}
