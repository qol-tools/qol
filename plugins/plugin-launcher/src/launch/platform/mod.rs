#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::launch_app;
#[cfg(target_os = "macos")]
pub(super) use macos::launch_app;
#[cfg(target_os = "windows")]
pub(super) use windows::launch_app;
