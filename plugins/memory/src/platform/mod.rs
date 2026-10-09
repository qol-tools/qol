#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod support;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(
    test,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub(crate) use fallback::open_for_times;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{current_support, run_dir_name};
#[cfg(all(test, target_os = "linux"))]
pub(crate) use linux::open_for_times;
#[cfg(target_os = "linux")]
pub(crate) use linux::{current_support, run_dir_name};
#[cfg(all(test, target_os = "macos"))]
pub(crate) use macos::open_for_times;
#[cfg(target_os = "macos")]
pub(crate) use macos::{current_support, run_dir_name};
pub(crate) use support::PlatformSupport;
#[cfg(all(test, target_os = "windows"))]
pub(crate) use windows::open_for_times;
#[cfg(target_os = "windows")]
pub(crate) use windows::{current_support, run_dir_name};
