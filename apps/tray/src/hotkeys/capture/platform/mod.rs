#[cfg(any(target_os = "windows", test))]
mod hook_rules;
#[cfg_attr(target_os = "linux", allow(dead_code))]
mod key_matcher;
#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    cancel_recording, install, release_held_keys, start_recording, KEEP_REGISTERED_ONE_SHOTS,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    cancel_recording, install, release_held_keys, start_recording, KEEP_REGISTERED_ONE_SHOTS,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    cancel_recording, install, release_held_keys, start_recording, KEEP_REGISTERED_ONE_SHOTS,
};
