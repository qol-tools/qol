use anyhow::Result;
use std::sync::Arc;

use crate::daemon::EventBus;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    linux::download_and_install(events).await
}

#[cfg(target_os = "macos")]
pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    macos::download_and_install(events).await
}

#[cfg(target_os = "windows")]
pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    windows::download_and_install(events).await
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("updates::platform::download_and_install is not implemented for this target OS");
