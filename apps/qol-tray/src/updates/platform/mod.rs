use anyhow::Result;
use std::sync::Arc;

use crate::daemon::EventBus;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
mod install_kind;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as active;
#[cfg(target_os = "linux")]
use linux as active;
#[cfg(target_os = "macos")]
use macos as active;
#[cfg(target_os = "windows")]
use windows as active;

pub(crate) use install_kind::InstallKind;

pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    active::download_and_install(events).await
}

fn detect_install_kind() -> InstallKind {
    active::detect_install_kind()
}
