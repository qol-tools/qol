#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod native_tools;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_common;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::{apply_theme, native_available, prewarm, request, run, show_toast, stop};
#[cfg(target_os = "linux")]
pub(super) use linux::{apply_theme, native_available, prewarm, request, run, show_toast, stop};
#[cfg(target_os = "macos")]
pub(super) use macos::{apply_theme, native_available, prewarm, request, run, show_toast, stop};
#[cfg(target_os = "windows")]
pub(super) use windows::{apply_theme, native_available, prewarm, request, run, show_toast, stop};
