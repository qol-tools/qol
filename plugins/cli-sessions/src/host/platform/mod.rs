#[cfg(not(target_os = "windows"))]
mod kitty;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(target_os = "windows"))]
pub(super) use kitty::{console_probe, system};
#[cfg(target_os = "windows")]
pub(super) use windows::{console_probe, system};
