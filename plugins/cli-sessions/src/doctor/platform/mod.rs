use std::path::PathBuf;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as selected;
#[cfg(target_os = "linux")]
use linux as selected;
#[cfg(target_os = "macos")]
use macos as selected;
#[cfg(target_os = "windows")]
use windows as selected;

pub(super) fn inspect() -> PlatformInspection {
    selected::inspect()
}

pub(super) struct PlatformInspection {
    pub(super) name: &'static str,
    pub(super) supported: bool,
    pub(super) kitten: Option<PathBuf>,
    pub(super) console: bool,
}
