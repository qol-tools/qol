#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod gpui_host;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod native_tools;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux::process_elapsed_ms;
#[cfg(target_os = "macos")]
use macos::process_elapsed_ms;
#[cfg(target_os = "windows")]
use windows::process_elapsed_ms;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::{
    apply_theme, native_available, plugins_changed, prewarm, request, run, show_toast, stop,
    wait_until_ready,
};
#[cfg(target_os = "linux")]
pub(super) use linux::{
    apply_theme, native_available, plugins_changed, prewarm, request, run, show_toast, stop,
    wait_until_ready,
};
#[cfg(target_os = "macos")]
pub(super) use macos::{
    apply_theme, native_available, plugins_changed, prewarm, request, run, show_toast, stop,
    wait_until_ready,
};
#[cfg(target_os = "windows")]
pub(super) use windows::{
    apply_theme, native_available, plugins_changed, prewarm, request, run, show_toast, stop,
    wait_until_ready,
};
