use super::{Permission, PermissionState};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::Permissions;
#[cfg(target_os = "macos")]
pub(super) use macos::Permissions;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use unsupported::Permissions;
#[cfg(target_os = "windows")]
pub(super) use windows::Permissions;

pub(super) trait PermissionsApi {
    fn status(&self, permission: Permission) -> PermissionState;
    fn request(&self, permission: Permission) -> PermissionState;
}
