#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{availability, lock_screen};
#[cfg(target_os = "macos")]
pub use macos::{availability, lock_screen};
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use unsupported::{availability, lock_screen};
#[cfg(target_os = "windows")]
pub use windows::{availability, lock_screen};
