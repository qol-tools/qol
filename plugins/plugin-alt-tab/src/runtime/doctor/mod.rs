mod endpoint;
mod platform;

use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::json;

const CHECK_IDS: [&str; 4] = [
    "platform_supported",
    "config_readable",
    "display_backend",
    "daemon_endpoint",
];

pub(super) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify Alt Tab declares a picker backend for the current platform.",
            || Ok(platform_supported_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Read and validate the typed Alt Tab config without changing it.",
            || Ok(config_readable_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Inspect display-session metadata without opening a display connection.",
            || Ok(display_backend_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Inspect daemon socket path metadata without connecting or binding.",
            || Ok(endpoint::check()),
        ),
    ]
}

#[cfg(test)]
pub(super) fn check_ids() -> &'static [&'static str] {
    &CHECK_IDS
}

fn platform_supported_check() -> DoctorCheckResult {
    let inspection = platform::inspect();
    let details = platform_details(&inspection);
    if inspection.supported {
        return DoctorCheckResult::ok(
            CHECK_IDS[0],
            format!(
                "{} has a declared Alt Tab picker backend.",
                inspection.platform
            ),
        )
        .with_details(details);
    }
    DoctorCheckResult::fail(
        CHECK_IDS[0],
        format!(
            "{} does not have a declared Alt Tab picker backend.",
            inspection.platform
        ),
    )
    .with_fix("Run Alt Tab on a platform declared by its plugin manifest.")
    .with_details(details)
}

fn config_readable_check() -> DoctorCheckResult {
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(CHECK_IDS[1], error.to_string())
                .with_fix("Repair or remove the invalid Alt Tab config file");
        }
    };
    let source = inspection
        .source
        .as_ref()
        .map(|path| path.display().to_string());
    let message = source.as_ref().map_or_else(
        || "No config file found; typed contract defaults are valid.".to_string(),
        |path| format!("Config at {path} is readable and matches the typed contract."),
    );
    DoctorCheckResult::ok(CHECK_IDS[1], message).with_details(json!({
        "source": source,
        "inspection": "read_only",
    }))
}

fn display_backend_check() -> DoctorCheckResult {
    let inspection = platform::inspect();
    let details = platform_details(&inspection);
    if !inspection.supported {
        return DoctorCheckResult::fail(
            CHECK_IDS[2],
            format!(
                "No display backend is available for {}.",
                inspection.platform
            ),
        )
        .with_fix("Run Alt Tab on a platform declared by its plugin manifest.")
        .with_details(details);
    }
    if !inspection.display_ready {
        return DoctorCheckResult::fail(
            CHECK_IDS[2],
            "The X11 backend is selected, but DISPLAY is not set.",
        )
        .with_fix("Run Alt Tab in an X11 or XWayland graphical session.")
        .with_details(details);
    }
    DoctorCheckResult::ok(
        CHECK_IDS[2],
        format!(
            "The {} display backend is selected from environment metadata; no connection was opened.",
            inspection.backend
        ),
    )
    .with_details(details)
}

fn platform_details(inspection: &platform::Inspection) -> serde_json::Value {
    json!({
        "platform": inspection.platform,
        "supported": inspection.supported,
        "backend": inspection.backend,
        "display_ready": inspection.display_ready,
        "display_env_set": inspection.display_env_set,
        "wayland_env_set": inspection.wayland_env_set,
        "session_type": inspection.session_type.as_deref(),
        "display_connected": false,
        "window_discovery_run": false,
        "preview_capture_run": false,
    })
}
