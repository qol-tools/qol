use std::path::{Path, PathBuf};

use crate::{DiskSpace, LinuxDisplayBackend, PlatformCapabilities};

#[cfg(all(
    not(unix),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
mod fallback;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
mod fallback_unix;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(
    target_os = "linux",
    all(
        unix,
        not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
    )
))]
mod statvfs;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(
    not(unix),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub(crate) use fallback::Platform;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub(crate) use fallback_unix::Platform;
#[cfg(target_os = "linux")]
pub(crate) use linux::Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Platform;

pub(crate) trait PlatformApi {
    fn linux_display_backend(&self) -> LinuxDisplayBackend;
    fn current_capabilities(&self) -> PlatformCapabilities;
    fn disk_space(&self, path: &Path) -> std::io::Result<DiskSpace>;

    fn launch_working_dir(&self) -> Option<PathBuf> {
        dirs::home_dir().or_else(|| std::env::current_dir().ok())
    }
}

pub(super) const FULL_CAPABILITIES: PlatformCapabilities = PlatformCapabilities {
    can_global_hotkey: true,
    can_focus_popup: true,
    can_clipboard_monitor: true,
    can_window_positioning: true,
};
