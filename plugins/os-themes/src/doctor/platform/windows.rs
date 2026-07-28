use super::{CurrentThemeMetadata, GsettingsMetadata, PlatformMetadata, SessionMetadata};

pub(super) fn inspect() -> PlatformMetadata {
    PlatformMetadata {
        platform: "Windows",
        supported: false,
        gsettings: GsettingsMetadata {
            path: None,
            executable: false,
            issue: Some("gsettings is not an OS Themes dependency on Windows".to_string()),
        },
        session: SessionMetadata {
            desktop: None,
            session_type: None,
            display_available: false,
            wayland_available: false,
            dbus_available: false,
            desktop_backend: None,
            desktop_backend_supported: false,
        },
        current_theme: CurrentThemeMetadata { gtk_theme: None },
    }
}
