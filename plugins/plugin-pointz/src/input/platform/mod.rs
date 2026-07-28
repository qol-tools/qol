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

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::inspect_readiness;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::platform_support;
#[cfg(target_os = "linux")]
pub(super) use linux::inspect_readiness;
#[cfg(target_os = "linux")]
pub(super) use linux::platform_support;
#[cfg(target_os = "macos")]
pub(super) use macos::inspect_readiness;
#[cfg(target_os = "macos")]
pub(super) use macos::platform_support;
#[cfg(target_os = "windows")]
pub(super) use windows::inspect_readiness;
#[cfg(target_os = "windows")]
pub(super) use windows::platform_support;
