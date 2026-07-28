use std::path::{Path, PathBuf};

pub(super) trait ExecutionPlatform {
    fn resolve_candidate(
        plugin_dir: &Path,
        command_path: &Path,
        canonical_plugin_dir: &Path,
    ) -> Option<PathBuf>;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

pub(super) fn resolve_platform_candidate(
    plugin_dir: &Path,
    command_path: &Path,
    canonical_plugin_dir: &Path,
) -> Option<PathBuf> {
    Platform::resolve_candidate(plugin_dir, command_path, canonical_plugin_dir)
}
