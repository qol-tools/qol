use std::path::PathBuf;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlatformMetadata {
    pub platform: &'static str,
    pub supported: bool,
    pub gsettings: GsettingsMetadata,
    pub session: SessionMetadata,
    pub current_theme: CurrentThemeMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GsettingsMetadata {
    pub path: Option<PathBuf>,
    pub executable: bool,
    pub issue: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionMetadata {
    pub desktop: Option<String>,
    pub session_type: Option<String>,
    pub display_available: bool,
    pub wayland_available: bool,
    pub dbus_available: bool,
    pub desktop_backend: Option<&'static str>,
    pub desktop_backend_supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CurrentThemeMetadata {
    pub gtk_theme: Option<String>,
}

pub(super) fn inspect() -> PlatformMetadata {
    imp::inspect()
}
