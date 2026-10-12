use anyhow::Result;
use std::path::{Path, PathBuf};

use super::AutostartState;

trait AutostartOps {
    fn read_state(&self) -> Result<AutostartState>;
    fn write_target(&self, binary: &Path) -> Result<()>;
    fn autostart_path(&self) -> Result<PathBuf>;
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
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

pub(super) fn read_state() -> Result<AutostartState> {
    Platform.read_state()
}

pub(super) fn write_target(binary: &Path) -> Result<()> {
    Platform.write_target(binary)
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    Platform.autostart_path()
}
