use anyhow::Result;
use std::path::{Path, PathBuf};

pub(crate) trait AutostartOps {
    fn read_target(&self) -> Result<Option<PathBuf>>;
    fn write_target(&self, binary: &Path) -> Result<()>;
    fn autostart_path(&self) -> Result<PathBuf>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(any(target_os = "windows", test))]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use unsupported::Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Platform;

pub(super) fn read_target() -> Result<Option<PathBuf>> {
    Platform.read_target()
}

pub(super) fn write_target(binary: &Path) -> Result<()> {
    Platform.write_target(binary)
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    Platform.autostart_path()
}
