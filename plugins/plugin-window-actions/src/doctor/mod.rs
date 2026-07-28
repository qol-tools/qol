use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::json;

const CHECK_IDS: [&str; 4] = [
    "platform_supported",
    "config_readable",
    "required_binaries",
    "restore_state",
];

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify the current platform has a declared Window Actions backend.",
            || Ok(crate::platform::platform_supported_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Read and validate the typed plugin config without changing it.",
            || Ok(config_readable_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Inspect required platform tools or libraries without executing window operations.",
            || Ok(crate::platform::required_binaries_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Inspect existing minimized-window state metadata without changing it.",
            || Ok(restore_state_check()),
        ),
    ]
}

#[cfg(test)]
pub(crate) fn check_ids() -> &'static [&'static str] {
    &CHECK_IDS
}

fn config_readable_check() -> DoctorCheckResult {
    let inspection = match crate::config::inspect_config() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail("config_readable", error.to_string())
                .with_fix("Repair or remove the invalid Window Actions config file");
        }
    };

    let message = match inspection.source {
        Some(path) => format!(
            "Config at {} is readable and matches the typed contract",
            path.display()
        ),
        None => "No config file found; typed contract defaults are valid".to_string(),
    };
    DoctorCheckResult::ok("config_readable", message)
}

fn restore_state_check() -> DoctorCheckResult {
    restore_state_result(&crate::platform::state_file_path())
}

fn restore_state_result(path: &Path) -> DoctorCheckResult {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return DoctorCheckResult::ok(
                "restore_state",
                "No minimized-window state file exists; there is nothing to restore",
            );
        }
        Err(error) => {
            return DoctorCheckResult::fail(
                "restore_state",
                format!("Minimized-window state metadata cannot be read: {error}"),
            )
            .with_fix("Make the Window Actions runtime directory readable");
        }
    };
    if metadata.file_type().is_symlink() {
        return DoctorCheckResult::warn(
            "restore_state",
            format!(
                "Minimized-window state path {} is a symbolic link",
                path.display()
            ),
        )
        .with_fix("Replace the symbolic link with a regular state file");
    }
    if !metadata.is_file() {
        return DoctorCheckResult::fail(
            "restore_state",
            format!(
                "Minimized-window state path {} is not a regular file",
                path.display()
            ),
        )
        .with_fix("Remove the invalid state path");
    }
    if metadata.permissions().readonly() {
        return DoctorCheckResult::fail(
            "restore_state",
            format!(
                "Minimized-window state file {} is read-only",
                path.display()
            ),
        )
        .with_fix("Make the state file writable or remove it");
    }

    DoctorCheckResult::ok(
        "restore_state",
        format!(
            "Minimized-window state file metadata is available ({} bytes)",
            metadata.len()
        ),
    )
    .with_details(json!({
        "bytes": metadata.len(),
        "file_type": "regular",
        "content_read": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::DoctorStatus;

    #[test]
    fn missing_restore_state_is_healthy_and_never_created() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing-state");

        let result = restore_state_result(&missing);

        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(!missing.exists());
    }
}
