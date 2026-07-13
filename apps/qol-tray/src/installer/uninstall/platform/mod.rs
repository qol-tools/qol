use anyhow::Result;

use super::model::{ProcessTargets, UninstallContext};

pub(super) trait PlatformOps {
    fn context(&self) -> Result<UninstallContext>;
    fn managed_processes(&self) -> Vec<crate::plugins::daemon_tracker::ManagedProcess>;
    fn stop_processes(&self, targets: &ProcessTargets) -> Result<()>;
    fn refresh_desktop_caches(&self, context: &UninstallContext) -> Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::Platform;
#[cfg(target_os = "macos")]
pub(super) use macos::Platform;
#[cfg(target_os = "windows")]
pub(super) use windows::Platform;
