#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    execute_action, permissions_check, platform_supported_check, required_binaries_check,
    state_file_path, GlideController, DIAGNOSTIC_ACTIONS,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    execute_action, permissions_check, platform_supported_check, required_binaries_check,
    state_file_path, GlideController, DIAGNOSTIC_ACTIONS,
};
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) use unsupported::{
    execute_action, permissions_check, platform_supported_check, required_binaries_check,
    state_file_path, GlideController, DIAGNOSTIC_ACTIONS,
};
