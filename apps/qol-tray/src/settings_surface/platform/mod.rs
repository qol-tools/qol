#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_common;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) use unix_common::{request, run, stop};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn request(_plugin_id: &str) -> anyhow::Result<bool> {
    Ok(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn run(_plugin_id: String) -> anyhow::Result<()> {
    anyhow::bail!("native settings surfaces are unsupported on this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn stop() {}
