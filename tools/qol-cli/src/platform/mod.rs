use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use unsupported::Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Platform;

pub(crate) trait PlatformOps {
    fn os_name(&self) -> &'static str;
    fn exe_name(&self, name: &str) -> String;
    fn stop_qol_tray(&self) -> Result<()>;
    fn open_url(&self, url: &str);
}
