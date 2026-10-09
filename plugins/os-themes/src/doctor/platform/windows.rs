use super::{CurrentThemeMetadata, PlatformMetadata, SessionMetadata};

const DESKTOP: &str = "Windows";
const SERVICE_SESSION: &str = "Services";

pub(super) fn inspect() -> PlatformMetadata {
    let session_type = std::env::var("SESSIONNAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    PlatformMetadata {
        platform: "Windows",
        supported: true,
        gsettings: None,
        session: SessionMetadata {
            desktop: Some(DESKTOP.to_string()),
            display_available: interactive_session(session_type.as_deref()),
            session_type,
            wayland_available: false,
            dbus_available: None,
            desktop_backend: Some(DESKTOP),
            desktop_backend_supported: true,
        },
        current_theme: CurrentThemeMetadata { gtk_theme: None },
    }
}

fn interactive_session(session_name: Option<&str>) -> bool {
    session_name.is_some_and(|name| !name.eq_ignore_ascii_case(SERVICE_SESSION))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_session_table() {
        let cases = [
            (Some("Console"), true),
            (Some("RDP-Tcp#3"), true),
            (Some("Services"), false),
            (Some("services"), false),
            (None, false),
        ];
        for (name, expected) in cases {
            assert_eq!(interactive_session(name), expected, "name={name:?}");
        }
    }
}
