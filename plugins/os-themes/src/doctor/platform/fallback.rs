use super::{CurrentThemeMetadata, GsettingsMetadata, PlatformMetadata, SessionMetadata};

pub(super) fn inspect() -> PlatformMetadata {
    PlatformMetadata {
        platform: std::env::consts::OS,
        supported: false,
        gsettings: GsettingsMetadata {
            path: None,
            executable: false,
            issue: Some("os-themes is unsupported on this platform".to_string()),
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
