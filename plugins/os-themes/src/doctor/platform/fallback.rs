use super::{CurrentThemeMetadata, PlatformMetadata, SessionMetadata};

pub(super) fn inspect() -> PlatformMetadata {
    PlatformMetadata {
        platform: std::env::consts::OS,
        supported: false,
        gsettings: None,
        session: SessionMetadata {
            desktop: None,
            session_type: None,
            display_available: false,
            wayland_available: false,
            dbus_available: None,
            desktop_backend: None,
            desktop_backend_supported: false,
        },
        current_theme: CurrentThemeMetadata { gtk_theme: None },
    }
}
