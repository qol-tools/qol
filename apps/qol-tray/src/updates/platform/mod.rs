use anyhow::Result;
use std::sync::Arc;

use crate::daemon::EventBus;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("updates::platform::download_and_install is not implemented for this target OS");

#[cfg(target_os = "linux")]
pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    linux::download_and_install(events).await
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) async fn download_and_install(_events: Arc<EventBus>) -> Result<()> {
    open_latest_release_page()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn open_latest_release_page() -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", super::GITHUB_REPO);
    crate::paths::open_url(&url)?;
    Ok(())
}
