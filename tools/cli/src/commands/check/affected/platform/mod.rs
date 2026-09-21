#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::current as current_impl;
#[cfg(target_os = "linux")]
use linux::current as current_impl;
#[cfg(target_os = "macos")]
use macos::current as current_impl;
#[cfg(target_os = "windows")]
use windows::current as current_impl;

pub(super) fn current() -> anyhow::Result<super::Platform> {
    current_impl()
}
