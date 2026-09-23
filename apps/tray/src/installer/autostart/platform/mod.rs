use anyhow::Result;
use std::path::{Path, PathBuf};

trait AutostartOps {
    fn read_target(&self) -> Result<Option<PathBuf>>;
    fn write_target(&self, binary: &Path) -> Result<()>;
    fn autostart_path(&self) -> Result<PathBuf>;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "windows", test))]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

pub(super) fn read_target() -> Result<Option<PathBuf>> {
    Platform.read_target()
}

pub(super) fn write_target(binary: &Path) -> Result<()> {
    Platform.write_target(binary)
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    Platform.autostart_path()
}
