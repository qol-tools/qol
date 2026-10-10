use anyhow::Result;
use std::sync::{Arc, Mutex};

use crate::daemon::EventBus;
use crate::plugins::PluginManager;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod download;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
mod install_kind;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(any(target_os = "windows", test))]
mod windows_install_kind;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as active;
#[cfg(target_os = "linux")]
use linux as active;
#[cfg(target_os = "macos")]
use macos as active;
#[cfg(target_os = "windows")]
use windows as active;

pub(crate) use install_kind::InstallKind;

pub(super) async fn download_and_install(
    events: Arc<EventBus>,
    plugin_manager: Arc<Mutex<PluginManager>>,
) -> Result<()> {
    active::download_and_install(events, plugin_manager).await
}

pub(crate) fn detect_install_kind() -> InstallKind {
    active::detect_install_kind()
}
