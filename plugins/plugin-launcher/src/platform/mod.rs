#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported {
    use std::path::Path;

    pub fn launch_app(_path: &Path, _exec: &[String]) -> bool {
        false
    }

    pub fn open_path(_path: &Path) -> bool {
        false
    }

    pub fn activate_app(cx: &mut gpui::App) {
        cx.activate(true);
    }

    pub fn set_activation_policy() {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as imp;

use std::path::{Path, PathBuf};

pub fn launch_app(path: &Path, exec: &[String]) -> bool {
    imp::launch_app(path, exec)
}

pub fn open_path(path: &Path) -> bool {
    imp::open_path(path)
}

pub fn activate_app(cx: &mut gpui::App) {
    imp::activate_app(cx)
}

pub fn set_activation_policy() {
    imp::set_activation_policy()
}

fn launch_working_dir() -> Option<PathBuf> {
    dirs::home_dir().or_else(|| std::env::current_dir().ok())
}

// --- Platform capability detection (unchanged) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxDisplayBackend {
    X11,
    Wayland,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub can_global_hotkey: bool,
    pub can_focus_popup: bool,
    pub can_clipboard_monitor: bool,
    pub can_window_positioning: bool,
}

#[cfg(target_os = "linux")]
pub fn linux_display_backend() -> LinuxDisplayBackend {
    let session = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() || session == "wayland";
    if has_wayland {
        return LinuxDisplayBackend::Wayland;
    }
    let has_x11 = std::env::var_os("DISPLAY").is_some() || session == "x11";
    if has_x11 {
        return LinuxDisplayBackend::X11;
    }
    LinuxDisplayBackend::Unknown
}

#[cfg(not(target_os = "linux"))]
pub fn linux_display_backend() -> LinuxDisplayBackend {
    LinuxDisplayBackend::Unknown
}

pub fn current_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "linux")]
    {
        match linux_display_backend() {
            LinuxDisplayBackend::X11 => PlatformCapabilities {
                can_global_hotkey: true,
                can_focus_popup: true,
                can_clipboard_monitor: true,
                can_window_positioning: true,
            },
            LinuxDisplayBackend::Wayland => PlatformCapabilities {
                can_global_hotkey: false,
                can_focus_popup: true,
                can_clipboard_monitor: false,
                can_window_positioning: false,
            },
            LinuxDisplayBackend::Unknown => PlatformCapabilities {
                can_global_hotkey: false,
                can_focus_popup: false,
                can_clipboard_monitor: false,
                can_window_positioning: false,
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        PlatformCapabilities {
            can_global_hotkey: true,
            can_focus_popup: true,
            can_clipboard_monitor: true,
            can_window_positioning: true,
        }
    }
}
