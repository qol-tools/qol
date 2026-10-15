#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{
    backend, capture, keyboard, mouse, restore, set_level, set_switch, touchpad, values,
};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(super) use unsupported::{
    backend, capture, keyboard, mouse, restore, set_level, set_switch, touchpad, values,
};
#[cfg(target_os = "windows")]
pub(super) use windows::{
    backend, capture, keyboard, mouse, restore, set_level, set_switch, touchpad, values,
};
