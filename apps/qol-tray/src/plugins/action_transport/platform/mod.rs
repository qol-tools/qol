#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_common;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as imp;
#[cfg(target_os = "windows")]
use windows as imp;

use super::DaemonActionDispatch;
use std::path::Path;

pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    imp::dispatch_action(endpoint, action_id)
}
