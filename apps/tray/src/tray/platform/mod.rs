#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::request_shutdown;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback::{create_tray, run_app, PlatformTray};
#[cfg(target_os = "linux")]
pub(crate) use linux::request_shutdown;
#[cfg(target_os = "linux")]
pub use linux::{create_tray, run_app, PlatformTray};
#[cfg(target_os = "macos")]
pub(crate) use macos::request_shutdown;
#[cfg(target_os = "macos")]
pub use macos::{create_tray, run_app, PlatformTray};
#[cfg(target_os = "windows")]
pub(crate) use windows::request_shutdown;
#[cfg(target_os = "windows")]
pub use windows::{create_tray, run_app, PlatformTray};
