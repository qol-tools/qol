#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub(super) fn sync(entries: &[super::ResolvedEntry]) -> anyhow::Result<()> {
    imp::sync(entries)
}

pub(super) fn verify_target(resolved: &super::ResolvedEntry) -> anyhow::Result<()> {
    if resolved.target.is_file() {
        return Ok(());
    }
    anyhow::bail!(
        "launcher entry {} points at missing binary {}",
        resolved.entry.file_stem,
        resolved.target.display()
    )
}

pub(super) fn publish_synced() {
    imp::publish_synced();
}
