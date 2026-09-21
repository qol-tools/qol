mod fallback_non_unix;
#[cfg(unix)]
mod fallback_unix;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(
    not(unix),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
use fallback_non_unix as imp;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
use fallback_unix as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

trait RestartPlatformOps {
    fn binary_name() -> &'static str;
    fn exec_restart(binary: &std::path::Path) -> Result<(), String>;
}

pub(super) fn binary_name() -> &'static str {
    imp::Platform::binary_name()
}

pub(super) fn exec_restart(binary: &std::path::Path) -> Result<(), String> {
    imp::Platform::exec_restart(binary)
}

const _: fallback_non_unix::Platform = fallback_non_unix::Platform;
#[cfg(unix)]
const _: fallback_unix::Platform = fallback_unix::Platform;
