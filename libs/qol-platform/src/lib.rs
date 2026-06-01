use std::path::PathBuf;

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

pub fn launch_working_dir() -> Option<PathBuf> {
    dirs::home_dir().or_else(|| std::env::current_dir().ok())
}
