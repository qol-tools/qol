mod platform;

use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Result;
use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::{json, Value};

use crate::daemon::actions::CONFIG;

const CHECK_IDS: [&str; 5] = [
    "platform_supported",
    "config_readable",
    "required_binaries",
    "runtime_dirs",
    "daemon_endpoint",
];

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify the current platform is declared by CLI Sessions.",
            platform_supported_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Read and validate the typed config without changing config state.",
            config_readable_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Inspect Kitty remote-control client metadata without running it.",
            required_binaries_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Inspect session-state and snapshot path metadata without reading or creating them.",
            runtime_dirs_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[4],
            "Inspect the configured daemon endpoint without connecting to it.",
            daemon_endpoint_check,
        ),
    ]
}

#[cfg(test)]
pub(crate) fn check_ids() -> &'static [&'static str] {
    &CHECK_IDS
}

fn platform_supported_check() -> Result<DoctorCheckResult> {
    let inspection = platform::inspect();
    let result = if inspection.supported {
        DoctorCheckResult::ok(
            CHECK_IDS[0],
            format!(
                "{} is declared and has a CLI Sessions adapter.",
                inspection.name
            ),
        )
    } else {
        DoctorCheckResult::fail(
            CHECK_IDS[0],
            format!("{} is not declared by CLI Sessions.", inspection.name),
        )
        .with_fix("Run CLI Sessions on Linux or macOS.")
    };
    Ok(result.with_details(json!({
        "platform": inspection.name,
        "declared": inspection.supported,
        "inspection": "metadata_only",
    })))
}

fn config_readable_check() -> Result<DoctorCheckResult> {
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return Ok(DoctorCheckResult::fail(CHECK_IDS[1], error.to_string())
                .with_fix("Repair or remove the invalid CLI Sessions config file"));
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
    Ok(
        DoctorCheckResult::ok(CHECK_IDS[1], message).with_details(json!({
            "source": source,
            "corner": inspection.config.corner,
            "service_command_count": inspection.config.service_commands.len(),
            "inspection": "read_only",
        })),
    )
}

fn required_binaries_check() -> Result<DoctorCheckResult> {
    let inspection = platform::inspect();
    let details = json!({
        "platform": inspection.name,
        "kitten": inspection.kitten,
        "executed": false,
        "remote_control_probed": false,
    });
    if !inspection.supported {
        return Ok(DoctorCheckResult::fail(
            CHECK_IDS[2],
            "Kitty session integration is unavailable on this platform.",
        )
        .with_fix("Run CLI Sessions on Linux or macOS.")
        .with_details(details));
    }
    let Some(path) = inspection.kitten else {
        return Ok(DoctorCheckResult::fail(
            CHECK_IDS[2],
            "The `kitten` remote-control client is unavailable on PATH.",
        )
        .with_fix("Install Kitty and make its `kitten` executable available on PATH.")
        .with_details(details));
    };
    Ok(DoctorCheckResult::ok(
        CHECK_IDS[2],
        format!(
            "Kitty remote-control client metadata is available at {}; it was not executed.",
            path.display()
        ),
    )
    .with_details(details))
}

fn runtime_dirs_check() -> Result<DoctorCheckResult> {
    runtime_dirs_result([
        PathSpec::new(
            "session_state",
            crate::storage::paths::state_path(),
            ExpectedKind::File,
        ),
        PathSpec::new(
            "anomalies",
            crate::storage::paths::anomalies_dir(),
            ExpectedKind::Directory,
        ),
        PathSpec::new(
            "snapshots",
            crate::storage::paths::snapshots_dir(),
            ExpectedKind::Directory,
        ),
    ])
}

fn runtime_dirs_result<const N: usize>(specs: [PathSpec; N]) -> Result<DoctorCheckResult> {
    if specs.iter().any(|spec| spec.path.is_none()) {
        return Ok(DoctorCheckResult::fail(
            CHECK_IDS[3],
            "The CLI Sessions data directory cannot be resolved.",
        )
        .with_fix("Run with a valid platform user-data directory."));
    }

    let observations = specs.into_iter().map(PathSpec::observe).collect::<Vec<_>>();
    let invalid = observations
        .iter()
        .filter(|observation| {
            matches!(
                observation.state,
                PathState::WrongType | PathState::Unreadable(_)
            )
        })
        .count();
    let symlinks = observations
        .iter()
        .filter(|observation| observation.state == PathState::Symlink)
        .count();
    let missing = observations
        .iter()
        .filter(|observation| observation.state == PathState::Missing)
        .count();
    let details = json!({
        "paths": observations
            .iter()
            .map(PathObservation::details)
            .collect::<Vec<_>>(),
        "content_read": false,
        "created": false,
    });

    let result = if invalid > 0 {
        DoctorCheckResult::fail(
            CHECK_IDS[3],
            format!("{invalid} runtime path(s) have an unreadable or unexpected type."),
        )
        .with_fix("Repair or remove the reported CLI Sessions runtime paths.")
    } else if symlinks > 0 {
        DoctorCheckResult::warn(
            CHECK_IDS[3],
            format!("{symlinks} runtime path(s) are symbolic links."),
        )
        .with_fix("Replace symbolic links with plugin-owned runtime paths.")
    } else {
        DoctorCheckResult::ok(
            CHECK_IDS[3],
            format!(
                "Runtime path metadata is valid; {missing} path(s) will be created only by operational commands."
            ),
        )
    };
    Ok(result.with_details(details))
}

fn daemon_endpoint_check() -> Result<DoctorCheckResult> {
    let Some(path) = qol_plugin_daemon::daemon::socket_path(&CONFIG) else {
        return Ok(DoctorCheckResult::warn(
            CHECK_IDS[4],
            "No daemon socket path is injected for this standalone process.",
        )
        .with_fix("Run CLI Sessions through qol-tray to inject its daemon endpoint.")
        .with_details(json!({
            "path": null,
            "connected": false,
            "created": false,
        })));
    };
    endpoint_result(path)
}

fn endpoint_result(path: PathBuf) -> Result<DoctorCheckResult> {
    let observation = PathSpec::new("daemon_socket", Some(path), ExpectedKind::Endpoint).observe();
    let details = json!({
        "endpoint": observation.details(),
        "connected": false,
        "created": false,
    });
    let result = match &observation.state {
        PathState::Expected => DoctorCheckResult::ok(
            CHECK_IDS[4],
            "Daemon endpoint metadata is present; no connection was attempted.",
        ),
        PathState::Missing => DoctorCheckResult::warn(
            CHECK_IDS[4],
            "The configured daemon endpoint does not currently exist.",
        )
        .with_fix("Start CLI Sessions through qol-tray when resident monitoring is needed."),
        PathState::Symlink => DoctorCheckResult::warn(
            CHECK_IDS[4],
            "The configured daemon endpoint is a symbolic link.",
        )
        .with_fix("Use a plugin-owned daemon socket path."),
        PathState::WrongType => DoctorCheckResult::fail(
            CHECK_IDS[4],
            "The configured daemon endpoint has an unexpected filesystem type.",
        )
        .with_fix("Remove the stale endpoint before starting CLI Sessions."),
        PathState::Unreadable(error) => DoctorCheckResult::fail(
            CHECK_IDS[4],
            format!("The configured daemon endpoint cannot be inspected: {error}"),
        )
        .with_fix("Repair the endpoint path permissions."),
    };
    Ok(result.with_details(details))
}

#[derive(Clone, Copy)]
enum ExpectedKind {
    File,
    Directory,
    Endpoint,
}

struct PathSpec {
    label: &'static str,
    path: Option<PathBuf>,
    expected: ExpectedKind,
}

impl PathSpec {
    fn new(label: &'static str, path: Option<PathBuf>, expected: ExpectedKind) -> Self {
        Self {
            label,
            path,
            expected,
        }
    }

    fn observe(self) -> PathObservation {
        let path = self
            .path
            .expect("runtime path resolution checked before observation");
        observe_path(self.label, path, self.expected)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PathState {
    Expected,
    Missing,
    Symlink,
    WrongType,
    Unreadable(String),
}

struct PathObservation {
    label: &'static str,
    path: PathBuf,
    state: PathState,
    readonly: Option<bool>,
}

impl PathObservation {
    fn details(&self) -> Value {
        let (state, error) = match &self.state {
            PathState::Expected => ("expected", None),
            PathState::Missing => ("missing", None),
            PathState::Symlink => ("symlink", None),
            PathState::WrongType => ("wrong_type", None),
            PathState::Unreadable(error) => ("unreadable", Some(error.as_str())),
        };
        json!({
            "label": self.label,
            "path": self.path,
            "state": state,
            "readonly": self.readonly,
            "error": error,
        })
    }
}

fn observe_path(label: &'static str, path: PathBuf, expected: ExpectedKind) -> PathObservation {
    let (state, readonly) = match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => (PathState::Symlink, None),
        Ok(metadata) => {
            let matches = match expected {
                ExpectedKind::File => metadata.is_file(),
                ExpectedKind::Directory => metadata.is_dir(),
                ExpectedKind::Endpoint => !metadata.is_file() && !metadata.is_dir(),
            };
            (
                if matches {
                    PathState::Expected
                } else {
                    PathState::WrongType
                },
                Some(metadata.permissions().readonly()),
            )
        }
        Err(error) if error.kind() == ErrorKind::NotFound => (PathState::Missing, None),
        Err(error) => (PathState::Unreadable(error.to_string()), None),
    };
    PathObservation {
        label,
        path,
        state,
        readonly,
    }
}

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;

    use super::*;

    #[test]
    fn runtime_path_inspection_never_creates_missing_targets() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("sessions.json");
        let anomalies = root.path().join("anomalies");
        let snapshots = root.path().join("snapshots");

        let result = runtime_dirs_result([
            PathSpec::new("session_state", Some(state.clone()), ExpectedKind::File),
            PathSpec::new(
                "anomalies",
                Some(anomalies.clone()),
                ExpectedKind::Directory,
            ),
            PathSpec::new(
                "snapshots",
                Some(snapshots.clone()),
                ExpectedKind::Directory,
            ),
        ])
        .unwrap();

        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(!state.exists());
        assert!(!anomalies.exists());
        assert!(!snapshots.exists());
    }

    #[test]
    fn endpoint_inspection_never_creates_or_connects() {
        let root = tempfile::tempdir().unwrap();
        let endpoint = root.path().join("missing.sock");

        let result = endpoint_result(endpoint.clone()).unwrap();

        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(!endpoint.exists());
        assert_eq!(result.details.unwrap()["connected"], false);
    }
}
