#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::InputHandlerImpl;
#[cfg(target_os = "linux")]
pub(super) use linux::InputHandlerImpl;
#[cfg(target_os = "macos")]
pub(super) use macos::InputHandlerImpl;
#[cfg(target_os = "windows")]
pub(super) use windows::InputHandlerImpl;
