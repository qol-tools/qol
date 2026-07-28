mod platform;

use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::json;

const CHECK_IDS: [&str; 5] = [
    "platform_supported",
    "config_readable",
    "gsettings_metadata",
    "session_metadata",
    "current_theme_metadata",
];

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify the current platform has a declared OS Themes backend.",
            || Ok(platform_supported_result(&platform::inspect())),
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Read and deserialize plugin config without changing parse markers.",
            || Ok(config_readable_result()),
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Inspect gsettings executable metadata without starting a process.",
            || Ok(gsettings_metadata_result(&platform::inspect())),
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Inspect desktop-session metadata without opening a display or socket.",
            || Ok(session_metadata_result(&platform::inspect())),
        ),
        DoctorCheck::new(
            CHECK_IDS[4],
            "Report current-theme process metadata only when available without contacting settings services.",
            || Ok(current_theme_metadata_result(&platform::inspect())),
        ),
    ]
}

#[cfg(test)]
pub(crate) fn check_ids() -> &'static [&'static str] {
    &CHECK_IDS
}

fn platform_supported_result(metadata: &platform::PlatformMetadata) -> DoctorCheckResult {
    if metadata.supported {
        return DoctorCheckResult::ok(
            CHECK_IDS[0],
            format!(
                "{} is declared and has OS Themes platform adapters",
                metadata.platform
            ),
        )
        .with_details(json!({
            "platform": metadata.platform,
            "declared": true,
            "inspection": "metadata_only",
        }));
    }

    DoctorCheckResult::fail(
        CHECK_IDS[0],
        format!("{} is not declared by OS Themes", metadata.platform),
    )
    .with_fix("Run OS Themes on Linux")
    .with_details(json!({
        "platform": metadata.platform,
        "declared": false,
        "inspection": "metadata_only",
    }))
}

fn config_readable_result() -> DoctorCheckResult {
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(CHECK_IDS[1], error.to_string())
                .with_fix("Repair or remove the invalid OS Themes config file")
                .with_details(json!({
                    "inspection": "read_only",
                    "parse_markers_changed": false,
                }));
        }
    };

    let message = match inspection.source.as_ref() {
        Some(path) => format!(
            "Config at {} is readable and matches the typed contract",
            path.display()
        ),
        None => "No config file found; typed contract defaults are valid".to_string(),
    };
    DoctorCheckResult::ok(CHECK_IDS[1], message).with_details(json!({
        "source": inspection.source,
        "inspection": "read_only",
        "parse_markers_changed": false,
    }))
}

fn gsettings_metadata_result(metadata: &platform::PlatformMetadata) -> DoctorCheckResult {
    if !metadata.supported {
        return DoctorCheckResult::fail(
            CHECK_IDS[2],
            format!("gsettings is not used on {}", metadata.platform),
        )
        .with_fix("Run OS Themes on Linux")
        .with_details(gsettings_details(metadata));
    }

    if metadata.gsettings.executable {
        return DoctorCheckResult::ok(
            CHECK_IDS[2],
            format!(
                "Executable gsettings metadata is available at {}",
                metadata
                    .gsettings
                    .path
                    .as_ref()
                    .expect("executable metadata must have a path")
                    .display()
            ),
        )
        .with_details(gsettings_details(metadata));
    }

    let message = metadata
        .gsettings
        .issue
        .as_deref()
        .unwrap_or("gsettings was not found in PATH");
    DoctorCheckResult::fail(CHECK_IDS[2], message)
        .with_fix("Install the GLib gsettings command and make it executable in PATH")
        .with_details(gsettings_details(metadata))
}

fn gsettings_details(metadata: &platform::PlatformMetadata) -> serde_json::Value {
    json!({
        "path": metadata.gsettings.path,
        "executable": metadata.gsettings.executable,
        "issue": metadata.gsettings.issue,
        "inspection": "metadata_only",
        "process_started": false,
    })
}

fn session_metadata_result(metadata: &platform::PlatformMetadata) -> DoctorCheckResult {
    if !metadata.supported {
        return DoctorCheckResult::fail(
            CHECK_IDS[3],
            format!(
                "Desktop-session inspection is unavailable on {}",
                metadata.platform
            ),
        )
        .with_fix("Run OS Themes on Linux")
        .with_details(session_details(metadata));
    }

    let result = match (
        metadata.session.desktop.as_ref(),
        metadata.session.desktop_backend,
        metadata.session.desktop_backend_supported,
    ) {
        (_, Some(backend), true) => DoctorCheckResult::ok(
            CHECK_IDS[3],
            format!("{backend} desktop metadata selects an implemented theme backend"),
        ),
        (_, Some(backend), false) => DoctorCheckResult::fail(
            CHECK_IDS[3],
            format!("{backend} desktop metadata selects an unimplemented theme backend"),
        )
        .with_fix("Use a GNOME or Cinnamon desktop session"),
        (Some(_), None, _) => DoctorCheckResult::fail(
            CHECK_IDS[3],
            format!(
                "Desktop metadata {:?} does not select a supported theme backend",
                metadata.session.desktop
            ),
        )
        .with_fix("Run OS Themes in a GNOME or Cinnamon desktop session"),
        (None, None, _) => DoctorCheckResult::warn(
            CHECK_IDS[3],
            "XDG_CURRENT_DESKTOP is not present in process metadata",
        )
        .with_fix("Launch OS Themes from the graphical desktop session"),
    };

    if metadata.session.desktop_backend_supported
        && (!metadata.session.display_available || !metadata.session.dbus_available)
    {
        return DoctorCheckResult::warn(
            CHECK_IDS[3],
            "Desktop metadata is present but display or D-Bus session metadata is incomplete",
        )
        .with_fix("Launch OS Themes from a complete graphical desktop session")
        .with_details(session_details(metadata));
    }

    result.with_details(session_details(metadata))
}

fn session_details(metadata: &platform::PlatformMetadata) -> serde_json::Value {
    json!({
        "desktop": metadata.session.desktop,
        "session_type": metadata.session.session_type,
        "display_available": metadata.session.display_available,
        "wayland_available": metadata.session.wayland_available,
        "dbus_available": metadata.session.dbus_available,
        "desktop_backend": metadata.session.desktop_backend,
        "desktop_backend_supported": metadata.session.desktop_backend_supported,
        "inspection": "process_environment",
        "display_opened": false,
        "socket_connected": false,
    })
}

fn current_theme_metadata_result(metadata: &platform::PlatformMetadata) -> DoctorCheckResult {
    if !metadata.supported {
        return DoctorCheckResult::fail(
            CHECK_IDS[4],
            format!(
                "Current-theme metadata is unavailable on {}",
                metadata.platform
            ),
        )
        .with_fix("Run OS Themes on Linux")
        .with_details(current_theme_details(metadata, "unavailable"));
    }

    let Some(theme) = metadata.current_theme.gtk_theme.as_deref() else {
        return DoctorCheckResult::ok(
            CHECK_IDS[4],
            "Current theme was safely skipped because process metadata does not expose GTK_THEME",
        )
        .with_details(current_theme_details(metadata, "skipped"));
    };

    DoctorCheckResult::ok(
        CHECK_IDS[4],
        format!("GTK_THEME process metadata exposes {theme:?}"),
    )
    .with_details(current_theme_details(metadata, "observed"))
}

fn current_theme_details(
    metadata: &platform::PlatformMetadata,
    inspection: &'static str,
) -> serde_json::Value {
    json!({
        "gtk_theme": metadata.current_theme.gtk_theme,
        "source": "process_environment",
        "inspection": inspection,
        "gsettings_executed": false,
        "settings_service_contacted": false,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use qol_headless::DoctorStatus;

    use super::*;
    use crate::doctor::platform::{
        CurrentThemeMetadata, GsettingsMetadata, PlatformMetadata, SessionMetadata,
    };

    fn metadata() -> PlatformMetadata {
        PlatformMetadata {
            platform: "Linux",
            supported: true,
            gsettings: GsettingsMetadata {
                path: Some(PathBuf::from("/usr/bin/gsettings")),
                executable: true,
                issue: None,
            },
            session: SessionMetadata {
                desktop: Some("GNOME".to_string()),
                session_type: Some("wayland".to_string()),
                display_available: false,
                wayland_available: true,
                dbus_available: true,
                desktop_backend: Some("GNOME"),
                desktop_backend_supported: true,
            },
            current_theme: CurrentThemeMetadata { gtk_theme: None },
        }
    }

    #[test]
    fn check_ids_are_stable() {
        assert_eq!(
            checks().iter().map(DoctorCheck::id).collect::<Vec<_>>(),
            check_ids()
        );
    }

    #[test]
    fn current_theme_is_skipped_without_contacting_settings_services() {
        let metadata = metadata();

        let result = current_theme_metadata_result(&metadata);

        assert_eq!(result.status, DoctorStatus::Ok);
        let details = result.details.expect("current-theme details missing");
        assert_eq!(details["inspection"], "skipped");
        assert_eq!(details["gsettings_executed"], false);
        assert_eq!(details["settings_service_contacted"], false);
    }

    #[test]
    fn rendering_results_does_not_change_inspected_metadata() {
        let metadata = metadata();
        let original = metadata.clone();

        let _ = platform_supported_result(&metadata);
        let _ = gsettings_metadata_result(&metadata);
        let _ = session_metadata_result(&metadata);
        let _ = current_theme_metadata_result(&metadata);

        assert_eq!(metadata, original);
    }
}
