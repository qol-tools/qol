#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{facts, volume};
#[cfg(unix)]
pub(crate) use unix::{facts, volume};
#[cfg(target_os = "windows")]
pub(crate) use windows::{facts, volume};
