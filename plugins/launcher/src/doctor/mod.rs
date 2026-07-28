mod platform;

use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Result;
use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::{json, Value};

const CHECK_IDS: [&str; 5] = [
    "platform_supported",
    "config_readable",
    "required_binaries",
    "discovery_state",
    "daemon_endpoint",
];

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify the current platform has a declared Launcher backend.",
            platform_supported_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Read and validate the typed Launcher config without changing it.",
            config_readable_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Inspect required backend metadata without executing external programs.",
            required_binaries_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Inspect discovery-root and cache metadata without scanning or creating paths.",
            discovery_state_check,
        ),
        DoctorCheck::new(
            CHECK_IDS[4],
            "Inspect the daemon endpoint without connecting or binding.",
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
                "{} is declared with the {} discovery backend.",
                inspection.name, inspection.discovery_backend
            ),
        )
    } else {
        DoctorCheckResult::fail(
            CHECK_IDS[0],
            format!("{} is not declared by Launcher.", inspection.name),
        )
        .with_fix("Run Launcher on Linux or macOS.")
    };
    Ok(result.with_details(json!({
        "platform": inspection.name,
        "declared": inspection.supported,
        "discovery_backend": inspection.discovery_backend,
        "inspection": "metadata_only",
    })))
}

fn config_readable_check() -> Result<DoctorCheckResult> {
    let inspection = match crate::config::inspect_launcher_config() {
        Ok(inspection) => inspection,
        Err(error) => {
            return Ok(DoctorCheckResult::fail(CHECK_IDS[1], error.to_string())
                .with_fix("Repair or remove the invalid Launcher config file"));
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
            "ghost_opacity_configured": inspection.config.display.ghost_opacity.is_some(),
            "ghost_color_configured": inspection.config.display.ghost_debug_color.is_some(),
            "inspection": "read_only",
        })),
    )
}

fn required_binaries_check() -> Result<DoctorCheckResult> {
    let inspection = platform::inspect();
    let details = json!({
        "platform": inspection.name,
        "discovery_backend": inspection.discovery_backend,
        "fixed_helpers": null,
        "opener_resolution": "deferred_to_platform_default",
        "executed": false,
    });
    if !inspection.supported {
        return Ok(DoctorCheckResult::fail(
            CHECK_IDS[2],
            "Launcher has no executable backend on this platform.",
        )
        .with_fix("Run Launcher on Linux or macOS.")
        .with_details(details));
    }
    Ok(DoctorCheckResult::ok(
        CHECK_IDS[2],
        "Discovery requires no fixed helper; platform opener selection was not executed.",
    )
    .with_details(details))
}

fn discovery_state_check() -> Result<DoctorCheckResult> {
    discovery_state_result(crate::discovery::paths())
}

fn discovery_state_result(paths: crate::discovery::DiscoveryPaths) -> Result<DoctorCheckResult> {
    let mut observations = paths
        .application_roots
        .into_iter()
        .map(|path| MetadataObservation::new("application_root", path, ExpectedKind::Directory))
        .chain(
            paths
                .file_roots
                .into_iter()
                .map(|path| MetadataObservation::new("file_root", path, ExpectedKind::Directory)),
        )
        .collect::<Vec<_>>();
    if let Some(cache) = paths.cache {
        observations.push(MetadataObservation::new(
            "file_cache",
            cache,
            ExpectedKind::File,
        ));
    }

    let invalid = observations
        .iter()
        .filter(|observation| {
            matches!(
                observation.state,
                MetadataState::WrongType | MetadataState::Unreadable(_)
            )
        })
        .count();
    let symlinks = observations
        .iter()
        .filter(|observation| observation.state == MetadataState::Symlink)
        .count();
    let missing = observations
        .iter()
        .filter(|observation| observation.state == MetadataState::Missing)
        .count();
    let details = json!({
        "paths": observations
            .iter()
            .map(MetadataObservation::details)
            .collect::<Vec<_>>(),
        "missing": missing,
        "discovery_content_read": false,
        "discovery_scanned": false,
        "created": false,
    });
    let result = if invalid > 0 {
        DoctorCheckResult::warn(
            CHECK_IDS[3],
            format!("{invalid} discovery path(s) have unreadable or unexpected metadata."),
        )
        .with_fix("Repair or remove the reported Launcher discovery paths.")
    } else if symlinks > 0 {
        DoctorCheckResult::warn(
            CHECK_IDS[3],
            format!("{symlinks} discovery path(s) are symbolic links."),
        )
        .with_fix("Verify that the symbolic links resolve to trusted discovery paths.")
    } else {
        DoctorCheckResult::ok(
            CHECK_IDS[3],
            format!(
                "Discovery metadata is available; {missing} path(s) remain absent until operational use."
            ),
        )
    };
    Ok(result.with_details(details))
}

fn daemon_endpoint_check() -> Result<DoctorCheckResult> {
    let Some(path) = crate::app::socket_path() else {
        return Ok(DoctorCheckResult::warn(
            CHECK_IDS[4],
            "No daemon socket path is injected for this standalone process.",
        )
        .with_fix("Run Launcher through qol-tray to inject its daemon endpoint.")
        .with_details(json!({
            "path": null,
            "connected": false,
            "bound": false,
            "created": false,
        })));
    };
    daemon_endpoint_result(path)
}

fn daemon_endpoint_result(path: PathBuf) -> Result<DoctorCheckResult> {
    let observation = MetadataObservation::new("daemon_socket", path, ExpectedKind::Endpoint);
    let details = json!({
        "endpoint": observation.details(),
        "connected": false,
        "bound": false,
        "created": false,
    });
    let result = match &observation.state {
        MetadataState::Expected => DoctorCheckResult::ok(
            CHECK_IDS[4],
            "Daemon endpoint metadata is present; no connection was attempted.",
        ),
        MetadataState::Missing => DoctorCheckResult::warn(
            CHECK_IDS[4],
            "The configured daemon endpoint does not currently exist.",
        )
        .with_fix("Start Launcher through qol-tray when retained activation is needed."),
        MetadataState::Symlink => DoctorCheckResult::warn(
            CHECK_IDS[4],
            "The configured daemon endpoint is a symbolic link.",
        )
        .with_fix("Use a plugin-owned daemon socket path."),
        MetadataState::WrongType => DoctorCheckResult::fail(
            CHECK_IDS[4],
            "The configured daemon endpoint has an unexpected filesystem type.",
        )
        .with_fix("Remove the stale endpoint before starting Launcher."),
        MetadataState::Unreadable(error) => DoctorCheckResult::fail(
            CHECK_IDS[4],
            format!("The configured daemon endpoint cannot be inspected: {error}"),
        )
        .with_fix("Repair the endpoint path permissions."),
    };
    Ok(result.with_details(details))
}

#[derive(Clone, Copy)]
enum ExpectedKind {
    Directory,
    File,
    Endpoint,
}

#[derive(Debug, PartialEq, Eq)]
enum MetadataState {
    Expected,
    Missing,
    Symlink,
    WrongType,
    Unreadable(String),
}

struct MetadataObservation {
    label: &'static str,
    path: PathBuf,
    state: MetadataState,
    readonly: Option<bool>,
}

impl MetadataObservation {
    fn new(label: &'static str, path: PathBuf, expected: ExpectedKind) -> Self {
        let (state, readonly) = match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => (MetadataState::Symlink, None),
            Ok(metadata) => {
                let expected = match expected {
                    ExpectedKind::Directory => metadata.is_dir(),
                    ExpectedKind::File => metadata.is_file(),
                    ExpectedKind::Endpoint => !metadata.is_dir() && !metadata.is_file(),
                };
                (
                    if expected {
                        MetadataState::Expected
                    } else {
                        MetadataState::WrongType
                    },
                    Some(metadata.permissions().readonly()),
                )
            }
            Err(error) if error.kind() == ErrorKind::NotFound => (MetadataState::Missing, None),
            Err(error) => (MetadataState::Unreadable(error.to_string()), None),
        };
        Self {
            label,
            path,
            state,
            readonly,
        }
    }

    fn details(&self) -> Value {
        let (state, error) = match &self.state {
            MetadataState::Expected => ("expected", None),
            MetadataState::Missing => ("missing", None),
            MetadataState::Symlink => ("symlink", None),
            MetadataState::WrongType => ("wrong_type", None),
            MetadataState::Unreadable(error) => ("unreadable", Some(error.as_str())),
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

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;

    use super::*;

    #[test]
    fn discovery_metadata_check_never_scans_or_creates_paths() {
        let root = tempfile::tempdir().unwrap();
        let app_root = root.path().join("applications");
        let file_root = root.path().join("files");
        let cache = root.path().join("cache").join("launcher.tsv");

        let result = discovery_state_result(crate::discovery::DiscoveryPaths {
            application_roots: vec![app_root.clone()],
            file_roots: vec![file_root.clone()],
            cache: Some(cache.clone()),
        })
        .unwrap();

        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(!app_root.exists());
        assert!(!file_root.exists());
        assert!(!cache.exists());
        assert_eq!(result.details.unwrap()["discovery_scanned"], false);
    }

    #[test]
    fn endpoint_check_never_creates_connects_or_binds() {
        let root = tempfile::tempdir().unwrap();
        let endpoint = root.path().join("launcher.sock");

        let result = daemon_endpoint_result(endpoint.clone()).unwrap();

        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(!endpoint.exists());
        assert_eq!(result.details.as_ref().unwrap()["connected"], false);
        assert_eq!(result.details.unwrap()["bound"], false);
    }
}
