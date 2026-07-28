use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use qol_headless::DoctorCheckResult;
use serde_json::json;

pub(super) fn config_readable_check() -> DoctorCheckResult {
    let inspection = match crate::daemon::inspect_config() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail("config_readable", error.to_string())
                .with_fix("Repair or remove the invalid Task Runner config file");
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

pub(super) fn configured_apps_check() -> DoctorCheckResult {
    let inspection = match crate::daemon::inspect_config() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(
                "configured_apps",
                format!("Configured IDE paths cannot be inspected: {error}"),
            )
            .with_fix("Repair or remove the invalid Task Runner config file");
        }
    };
    configured_apps_result(&inspection.config)
}

fn configured_apps_result(config: &crate::daemon::Config) -> DoctorCheckResult {
    let configured = config.apps.keys().cloned().collect::<Vec<_>>();
    let available = configured
        .iter()
        .filter(|id| crate::daemon::find_executable(id, config).is_some())
        .cloned()
        .collect::<Vec<_>>();
    let unavailable = configured
        .iter()
        .filter(|id| !available.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    let details = json!({
        "configured": configured,
        "available": available,
        "unavailable": unavailable,
    });

    if configured.is_empty() {
        return DoctorCheckResult::fail("configured_apps", "No IDE launchers are configured")
            .with_fix("Configure at least one IDE launcher in Task Runner settings")
            .with_details(details);
    }
    if available.is_empty() {
        return DoctorCheckResult::fail(
            "configured_apps",
            format!(
                "None of the {} configured IDE launchers are available",
                configured.len()
            ),
        )
        .with_fix("Configure an executable path for an installed IDE")
        .with_details(details);
    }
    if !available.iter().any(|id| id == "idea") {
        return DoctorCheckResult::warn(
            "configured_apps",
            format!(
                "{} of {} configured IDE launchers are available, but the default 'idea' launcher is unavailable",
                available.len(),
                configured.len()
            ),
        )
        .with_fix("Configure the 'idea' launcher or always request an available app explicitly")
        .with_details(details);
    }

    DoctorCheckResult::ok(
        "configured_apps",
        format!(
            "{} of {} configured IDE launchers are available, including the default 'idea' launcher",
            available.len(),
            configured.len()
        ),
    )
    .with_details(details)
}

pub(super) fn temp_root_check() -> DoctorCheckResult {
    let inspection = match crate::daemon::inspect_config() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(
                "temp_root",
                format!("The checkout temp root cannot be inspected: {error}"),
            )
            .with_fix("Repair or remove the invalid Task Runner config file");
        }
    };
    temp_root_result(&inspection.config.temp_dir)
}

fn temp_root_result(path: &Path) -> DoctorCheckResult {
    if path.as_os_str().is_empty() {
        return DoctorCheckResult::fail("temp_root", "The checkout temp root is empty")
            .with_fix("Configure an absolute checkout temp directory");
    }

    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return DoctorCheckResult::warn(
                "temp_root",
                format!(
                    "Checkout temp root {} does not exist and will be created on first use",
                    path.display()
                ),
            )
            .with_fix(format!("Create {}", path.display()));
        }
        Err(error) => {
            return DoctorCheckResult::fail(
                "temp_root",
                format!(
                    "Checkout temp root {} cannot be inspected: {error}",
                    path.display()
                ),
            )
            .with_fix("Choose a readable checkout temp directory");
        }
    };
    if !metadata.is_dir() {
        return DoctorCheckResult::fail(
            "temp_root",
            format!("Checkout temp root {} is not a directory", path.display()),
        )
        .with_fix("Choose a directory for the checkout temp root");
    }
    if metadata.permissions().readonly() {
        return DoctorCheckResult::fail(
            "temp_root",
            format!("Checkout temp root {} is read-only", path.display()),
        )
        .with_fix("Choose a writable checkout temp directory");
    }
    if !path.is_absolute() {
        return DoctorCheckResult::warn(
            "temp_root",
            format!(
                "Checkout temp root {} is relative to the process directory",
                path.display()
            ),
        )
        .with_fix("Configure an absolute checkout temp directory");
    }

    DoctorCheckResult::ok(
        "temp_root",
        format!(
            "Checkout temp root {} is an available directory",
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::DoctorStatus;

    #[test]
    fn temp_root_inspection_never_creates_a_missing_directory() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");

        let result = temp_root_result(&missing);

        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(!missing.exists());
    }

    #[test]
    fn configured_apps_inspection_reads_executable_metadata_without_launching() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("idea");
        let launch_marker = root.path().join("launched");
        fs::write(
            &executable,
            "#!/bin/sh\ntouch \"$(dirname \"$0\")/launched\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = crate::daemon::Config::defaults();
        config.apps.retain(|id, _| id == "idea");
        config
            .apps
            .get_mut("idea")
            .expect("default idea config must exist")
            .paths = vec![executable.to_string_lossy().into_owned()];
        config.temp_dir = root.path().to_path_buf();

        let result = configured_apps_result(&config);

        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(!launch_marker.exists());
    }
}
