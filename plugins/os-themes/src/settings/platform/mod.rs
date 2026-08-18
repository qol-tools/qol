use anyhow::Result;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::Platform;
#[cfg(target_os = "linux")]
pub(super) use linux::Platform;
#[cfg(target_os = "macos")]
pub(super) use macos::Platform;
#[cfg(target_os = "windows")]
pub(super) use windows::Platform;

pub(super) trait SettingsPlatform {
    fn open(&self) -> Result<()>;
}
