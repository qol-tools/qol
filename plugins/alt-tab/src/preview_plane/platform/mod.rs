#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{hide_async, live_preview_replacement, prepare, show_async};
#[cfg(target_os = "linux")]
pub(crate) use linux::{hide_async, live_preview_replacement, prepare, show_async};
#[cfg(target_os = "macos")]
pub(crate) use macos::{hide_async, live_preview_replacement, prepare, show_async};
#[cfg(target_os = "windows")]
pub(crate) use windows::{hide_async, live_preview_replacement, prepare, show_async};
