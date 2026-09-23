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

pub(super) fn sync(
    entries: &[super::LauncherEntry],
    target: &std::path::Path,
) -> anyhow::Result<()> {
    imp::sync(entries, target)
}

pub(super) fn verify_target(
    entry: &super::LauncherEntry,
    target: &std::path::Path,
) -> anyhow::Result<()> {
    if target.is_file() {
        return Ok(());
    }
    anyhow::bail!(
        "launcher entry {} points at missing binary {}",
        entry.file_stem,
        target.display()
    )
}

pub(super) fn publish_synced() {
    imp::publish_synced();
}
