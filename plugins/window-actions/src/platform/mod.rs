#[cfg(any(target_os = "macos", target_os = "windows"))]
mod layout;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    execute_action, platform_supported_check, required_binaries_check, state_file_path,
    GlideController, DIAGNOSTIC_ACTIONS,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    execute_action, platform_supported_check, required_binaries_check, state_file_path,
    GlideController, DIAGNOSTIC_ACTIONS,
};
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use unsupported::{
    execute_action, platform_supported_check, required_binaries_check, state_file_path,
    GlideController, DIAGNOSTIC_ACTIONS,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    execute_action, platform_supported_check, required_binaries_check, state_file_path,
    GlideController, DIAGNOSTIC_ACTIONS,
};
