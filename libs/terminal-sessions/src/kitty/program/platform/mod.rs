use std::path::{Path, PathBuf};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use fallback as active;
#[cfg(target_os = "linux")]
use linux as active;
#[cfg(target_os = "macos")]
use macos as active;

pub(super) fn install_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    active::install_candidates(home)
}

pub(super) fn is_executable(path: &Path) -> bool {
    active::is_executable(path)
}
