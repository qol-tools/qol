use crate::cli::RunOptions;
use anyhow::Result;
use qol_headless::DoctorCheckResult;

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

trait GuestRunnerPlatform {
    fn run(&self, options: RunOptions) -> Result<()>;
    fn platform_check(&self) -> DoctorCheckResult;
    fn runtime_paths_check(&self) -> DoctorCheckResult;
}

pub(crate) fn run(options: RunOptions) -> Result<()> {
    Platform.run(options)
}

pub(crate) fn platform_check() -> DoctorCheckResult {
    Platform.platform_check()
}

pub(crate) fn runtime_paths_check() -> DoctorCheckResult {
    Platform.runtime_paths_check()
}
