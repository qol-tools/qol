use super::{CurrentThemeMetadata, PlatformMetadata, SessionMetadata};

const DESKTOP: &str = qol_platform::Os::Windows.label();

pub(super) fn inspect() -> PlatformMetadata {
    let session_type = std::env::var("SESSIONNAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    PlatformMetadata {
        platform: qol_platform::Os::Windows.label(),
        supported: true,
        gsettings: None,
        session: SessionMetadata {
            desktop: Some(DESKTOP.to_string()),
            display_available: qol_platform::native::session::interactive_session_id().is_some(),
            session_type,
            wayland_available: false,
            dbus_available: None,
            desktop_backend: Some(DESKTOP),
            desktop_backend_supported: true,
        },
        current_theme: CurrentThemeMetadata { gtk_theme: None },
    }
}
