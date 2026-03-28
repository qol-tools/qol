use anyhow::Result;
use std::path::{Path, PathBuf};

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
compile_error!("doctor platform implementation is required for this target OS");

pub(super) fn read_autostart_target() -> Result<Option<PathBuf>> {
    imp::read_autostart_target()
}

#[cfg(target_os = "macos")]
pub(super) fn install_marker_required(current_exe: &Path) -> bool {
    imp::install_marker_required(current_exe)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn install_marker_required(_current_exe: &Path) -> bool {
    true
}
