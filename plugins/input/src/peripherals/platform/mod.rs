#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{backend, keyboard, mouse, set_level, set_switch, touchpad};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(super) use unsupported::{backend, keyboard, mouse, set_level, set_switch, touchpad};
#[cfg(target_os = "windows")]
pub(super) use windows::{backend, keyboard, mouse, set_level, set_switch, touchpad};
