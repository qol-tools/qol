use qol_headless::DoctorCheckResult;
use serde_json::json;

pub(super) fn required_binaries_check() -> DoctorCheckResult {
    let Some(path) = crate::daemon::executable_on_path("git") else {
        return DoctorCheckResult::fail("required_binaries", "Git is unavailable on PATH")
            .with_fix("Repair the Git installation available on PATH");
    };

    DoctorCheckResult::ok(
        "required_binaries",
        format!("Git executable metadata is available at {}", path.display()),
    )
    .with_details(json!({
        "binary": "git",
        "path": path,
        "inspection": "metadata_only",
    }))
}

pub(super) fn runtime_assets_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "runtime_assets",
        format!(
            "The daemon is compiled into {}; no Python interpreter or packaged script is required",
            env!("QOL_PLUGIN_ID")
        ),
    )
    .with_details(json!({
        "daemon": "native-rust",
        "interpreter": null,
        "script": null,
    }))
}

pub(super) fn daemon_endpoint_check() -> DoctorCheckResult {
    let port = crate::daemon::daemon_port();
    DoctorCheckResult::ok(
        "daemon_endpoint",
        format!(
            "Health endpoint is configured at http://127.0.0.1:{port}/health; no connection was attempted"
        ),
    )
    .with_details(json!({
        "host": "127.0.0.1",
        "port": port,
        "path": "/health",
        "probed": false,
    }))
}
