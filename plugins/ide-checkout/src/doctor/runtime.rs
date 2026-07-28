use std::path::PathBuf;

use qol_headless::DoctorCheckResult;
use serde_json::json;

pub(super) fn required_binaries_check() -> DoctorCheckResult {
    let Some(path) = executable_on_path("git") else {
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
        "The daemon is compiled into task-runner; no Python interpreter or packaged script is required",
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

fn executable_on_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    executable_in_directories(program, std::env::split_paths(&paths))
}

fn executable_in_directories(
    program: &str,
    directories: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    directories
        .into_iter()
        .map(|directory| directory.join(program))
        .find(|candidate| crate::daemon::is_executable(candidate))
}

#[cfg(test)]
mod tests {
    use super::executable_in_directories;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn path_inspection_never_executes_the_candidate() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("git");
        let marker = root.path().join("launched");
        std::fs::write(
            &executable,
            "#!/bin/sh\ntouch \"$(dirname \"$0\")/launched\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();

        let found = executable_in_directories("git", [root.path().to_path_buf()]);

        assert_eq!(found.as_deref(), Some(executable.as_path()));
        assert!(!marker.exists());
    }
}
