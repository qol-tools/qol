use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{CurrentThemeMetadata, GsettingsMetadata, PlatformMetadata, SessionMetadata};
use crate::theme::platform::{classify_desktop, DesktopEnvironment};

pub(super) fn inspect() -> PlatformMetadata {
    let desktop = environment_value("XDG_CURRENT_DESKTOP");
    let session_type = environment_value("XDG_SESSION_TYPE");
    let display_available = environment_value("DISPLAY").is_some();
    let wayland_available = environment_value("WAYLAND_DISPLAY").is_some();
    let dbus_available = environment_value("DBUS_SESSION_BUS_ADDRESS").is_some();
    let (desktop_backend, desktop_backend_supported) = classify_session_desktop(desktop.as_deref());

    PlatformMetadata {
        platform: "Linux",
        supported: true,
        gsettings: inspect_gsettings(std::env::var_os("PATH")),
        session: SessionMetadata {
            desktop,
            session_type,
            display_available,
            wayland_available,
            dbus_available,
            desktop_backend,
            desktop_backend_supported,
        },
        current_theme: CurrentThemeMetadata {
            gtk_theme: environment_value("GTK_THEME"),
        },
    }
}

fn environment_value(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn classify_session_desktop(desktop: Option<&str>) -> (Option<&'static str>, bool) {
    let Some(desktop) = desktop else {
        return (None, false);
    };

    match classify_desktop(desktop) {
        DesktopEnvironment::Gnome => (Some("GNOME"), true),
        DesktopEnvironment::Cinnamon => (Some("Cinnamon"), true),
        DesktopEnvironment::Kde => (Some("KDE"), false),
        DesktopEnvironment::Unknown => (None, false),
    }
}

fn inspect_gsettings(path: Option<std::ffi::OsString>) -> GsettingsMetadata {
    let Some(path) = path else {
        return GsettingsMetadata {
            path: None,
            executable: false,
            issue: Some("PATH is unavailable; gsettings metadata cannot be inspected".to_string()),
        };
    };

    let candidates = std::env::split_paths(&path)
        .map(|directory| directory.join("gsettings"))
        .collect::<Vec<_>>();
    inspect_gsettings_candidates(&candidates)
}

fn inspect_gsettings_candidates(candidates: &[PathBuf]) -> GsettingsMetadata {
    let mut non_executable = None;
    for candidate in candidates {
        let Some(executable) = executable_metadata(candidate) else {
            continue;
        };
        if executable {
            return GsettingsMetadata {
                path: Some(candidate.clone()),
                executable: true,
                issue: None,
            };
        }
        if non_executable.is_none() {
            non_executable = Some(candidate.clone());
        }
    }

    match non_executable {
        Some(path) => GsettingsMetadata {
            issue: Some(format!(
                "gsettings exists at {} but is not executable",
                path.display()
            )),
            path: Some(path),
            executable: false,
        },
        None => GsettingsMetadata {
            path: None,
            executable: false,
            issue: Some("gsettings was not found in PATH".to_string()),
        },
    }
}

fn executable_metadata(path: &Path) -> Option<bool> {
    let metadata = fs::metadata(path).ok()?;
    Some(metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn desktop_classification_matches_operational_backends() {
        assert_eq!(
            classify_session_desktop(Some("ubuntu:GNOME")),
            (Some("GNOME"), true)
        );
        assert_eq!(
            classify_session_desktop(Some("X-Cinnamon")),
            (Some("Cinnamon"), true)
        );
        assert_eq!(classify_session_desktop(Some("KDE")), (Some("KDE"), false));
        assert_eq!(classify_session_desktop(Some("Hyprland")), (None, false));
        assert_eq!(classify_session_desktop(None), (None, false));
    }

    #[test]
    fn executable_inspection_does_not_change_file_metadata() {
        let executable = std::env::current_exe().expect("current executable unavailable");
        let before = fs::metadata(&executable).expect("current executable metadata unavailable");

        let result = inspect_gsettings_candidates(std::slice::from_ref(&executable));

        let after = fs::metadata(&executable).expect("current executable metadata unavailable");
        assert!(result.executable);
        assert_eq!(result.path.as_deref(), Some(executable.as_path()));
        assert_eq!(before.len(), after.len());
        assert_eq!(before.permissions().mode(), after.permissions().mode());
        assert_eq!(before.modified().ok(), after.modified().ok());
    }
}
