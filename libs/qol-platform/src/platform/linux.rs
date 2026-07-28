use std::path::Path;

use crate::{DiskSpace, LinuxDisplayBackend, PlatformCapabilities};

use super::{PlatformApi, FULL_CAPABILITIES};

pub(crate) struct Platform;

impl PlatformApi for Platform {
    fn linux_display_backend(&self) -> LinuxDisplayBackend {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        detect_display_backend(
            &session,
            std::env::var_os("WAYLAND_DISPLAY").is_some(),
            std::env::var_os("DISPLAY").is_some(),
        )
    }

    fn current_capabilities(&self) -> PlatformCapabilities {
        capabilities_for(self.linux_display_backend())
    }

    fn disk_space(&self, path: &Path) -> std::io::Result<DiskSpace> {
        super::statvfs::disk_space(path)
    }
}

fn detect_display_backend(
    session: &str,
    has_wayland_display: bool,
    has_x11_display: bool,
) -> LinuxDisplayBackend {
    let session = session.to_ascii_lowercase();
    if has_wayland_display || session == "wayland" {
        return LinuxDisplayBackend::Wayland;
    }
    if has_x11_display || session == "x11" {
        return LinuxDisplayBackend::X11;
    }
    LinuxDisplayBackend::Unknown
}

fn capabilities_for(backend: LinuxDisplayBackend) -> PlatformCapabilities {
    match backend {
        LinuxDisplayBackend::X11 => FULL_CAPABILITIES,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_backend_prioritizes_wayland_then_x11() {
        let cases = [
            ("WAYLAND", false, false, LinuxDisplayBackend::Wayland),
            ("X11", false, false, LinuxDisplayBackend::X11),
            ("", true, true, LinuxDisplayBackend::Wayland),
            ("", false, true, LinuxDisplayBackend::X11),
            ("", false, false, LinuxDisplayBackend::Unknown),
        ];

        for (session, has_wayland, has_x11, expected) in cases {
            assert_eq!(
                detect_display_backend(session, has_wayland, has_x11),
                expected,
                "session={session}, has_wayland={has_wayland}, has_x11={has_x11}"
            );
        }
    }

    #[test]
    fn capabilities_follow_display_backend() {
        let cases = [
            (LinuxDisplayBackend::X11, FULL_CAPABILITIES),
            (
                LinuxDisplayBackend::Wayland,
                PlatformCapabilities {
                    can_global_hotkey: false,
                    can_focus_popup: true,
                    can_clipboard_monitor: false,
                    can_window_positioning: false,
                },
            ),
            (
                LinuxDisplayBackend::Unknown,
                PlatformCapabilities {
                    can_global_hotkey: false,
                    can_focus_popup: false,
                    can_clipboard_monitor: false,
                    can_window_positioning: false,
                },
            ),
        ];

        for (backend, expected) in cases {
            assert_eq!(capabilities_for(backend), expected, "backend={backend:?}");
        }
    }
}
