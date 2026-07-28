#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use fallback::inspect;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use fallback::inspect_permissions;
#[cfg(target_os = "linux")]
pub(super) use linux::inspect;
#[cfg(target_os = "linux")]
pub(super) use linux::inspect_permissions;
#[cfg(target_os = "macos")]
pub(super) use macos::inspect;
#[cfg(target_os = "macos")]
pub(super) use macos::inspect_permissions;

pub(super) struct Inspection {
    pub platform: &'static str,
    pub supported: bool,
    pub backend: &'static str,
    pub display_ready: bool,
    pub display_env_set: bool,
    pub wayland_env_set: bool,
    pub session_type: Option<String>,
}

pub(super) struct PermissionInspection {
    pub platform: &'static str,
    pub supported: bool,
    pub accessibility_trusted: Option<bool>,
    pub screen_recording_trusted: Option<bool>,
}
