use super::PermissionStatus;
use crate::plugins::manifest::Capabilities;
use std::collections::HashMap;

pub(super) trait PermissionPlatform {
    fn check_plugin_permissions(capabilities: &Capabilities) -> HashMap<String, PermissionStatus>;
    fn check_permission(name: &str) -> Option<PermissionStatus>;
    fn request_permission(name: &str) -> Option<PermissionStatus>;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

pub(super) fn check_plugin_permissions(
    capabilities: &Capabilities,
) -> HashMap<String, PermissionStatus> {
    Platform::check_plugin_permissions(capabilities)
}

pub(super) fn check_permission(name: &str) -> Option<PermissionStatus> {
    Platform::check_permission(name)
}

pub(super) fn request_permission(name: &str) -> Option<PermissionStatus> {
    Platform::request_permission(name)
}
