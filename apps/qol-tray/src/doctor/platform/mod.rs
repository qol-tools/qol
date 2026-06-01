use std::path::Path;

pub(crate) trait DoctorPlatformOps {
    fn install_marker_required(&self, current_exe: &Path) -> bool;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use unsupported::Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Platform;

pub(super) fn install_marker_required(current_exe: &Path) -> bool {
    Platform.install_marker_required(current_exe)
}
