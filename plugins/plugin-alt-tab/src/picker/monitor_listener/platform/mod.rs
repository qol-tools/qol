#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::data_refresh_listener_loop;
#[cfg(target_os = "macos")]
pub(super) use macos::data_refresh_listener_loop;
#[cfg(target_os = "windows")]
pub(super) use windows::data_refresh_listener_loop;
