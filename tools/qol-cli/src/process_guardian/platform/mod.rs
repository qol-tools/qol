use std::path::PathBuf;

use anyhow::Result;

#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;

trait GuardianPlatform {
    fn guardian_executable(&self) -> Result<PathBuf>;
}

pub(super) fn guardian_executable() -> Result<PathBuf> {
    Platform.guardian_executable()
}
