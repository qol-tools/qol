use std::io::ErrorKind;
use std::path::Path;

use qol_headless::DoctorCheckResult;
use serde_json::json;

const CHECK_ID: &str = "daemon_endpoint";

pub(super) fn check() -> DoctorCheckResult {
    let Some(path) = qol_plugin_daemon::daemon::socket_path(&super::super::daemon::CONFIG) else {
        return DoctorCheckResult::warn(
            CHECK_ID,
            "No daemon socket path is injected for this process.",
        )
        .with_fix("Run Alt Tab through qol-tray to inject its daemon endpoint.")
        .with_details(json!({
            "path": null,
            "state": "not_configured",
            "connected": false,
            "bound": false,
            "created": false,
        }));
    };
    result(&path)
}

fn result(path: &Path) -> DoctorCheckResult {
    let state = inspect(path);
    let details = json!({
        "path": path,
        "state": state.as_str(),
        "connected": false,
        "bound": false,
        "created": false,
    });
    match state {
        EndpointState::Endpoint => DoctorCheckResult::ok(
            CHECK_ID,
            "Daemon endpoint metadata is present; no connection was attempted.",
        )
        .with_details(details),
        EndpointState::Missing => DoctorCheckResult::warn(
            CHECK_ID,
            "The configured daemon endpoint does not currently exist.",
        )
        .with_fix("Start Alt Tab through qol-tray when the retained picker is needed.")
        .with_details(details),
        EndpointState::Symlink => DoctorCheckResult::warn(
            CHECK_ID,
            "The configured daemon endpoint is a symbolic link.",
        )
        .with_fix("Use a plugin-owned daemon socket path.")
        .with_details(details),
        EndpointState::WrongType => DoctorCheckResult::fail(
            CHECK_ID,
            "The configured daemon endpoint has an unexpected filesystem type.",
        )
        .with_fix("Remove the stale endpoint before starting Alt Tab.")
        .with_details(details),
        EndpointState::Unreadable(error) => DoctorCheckResult::fail(
            CHECK_ID,
            format!("The configured daemon endpoint cannot be inspected: {error}"),
        )
        .with_fix("Repair the endpoint path permissions.")
        .with_details(details),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum EndpointState {
    Endpoint,
    Missing,
    Symlink,
    WrongType,
    Unreadable(String),
}

impl EndpointState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Endpoint => "endpoint",
            Self::Missing => "missing",
            Self::Symlink => "symlink",
            Self::WrongType => "wrong_type",
            Self::Unreadable(_) => "unreadable",
        }
    }
}

fn inspect(path: &Path) -> EndpointState {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => EndpointState::Symlink,
        Ok(metadata) if metadata.is_file() || metadata.is_dir() => EndpointState::WrongType,
        Ok(_) => EndpointState::Endpoint,
        Err(error) if error.kind() == ErrorKind::NotFound => EndpointState::Missing,
        Err(error) => EndpointState::Unreadable(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
