mod platform;

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use platform::RevealPlan;

/// Opens a path or URL with the operating system's default application without
/// blocking the caller on the launched application.
pub fn open_with_default_app(target: impl AsRef<OsStr>) -> io::Result<()> {
    platform::open(target.as_ref())
}

pub fn open_with_default_app_checked(target: impl AsRef<OsStr>) -> io::Result<()> {
    platform::open_checked(target.as_ref())
}

/// Asks the running tray to focus this plugin in the unified settings window.
pub fn open_plugin_settings_via_tray(plugin_id: &str) -> io::Result<()> {
    let path = qol_conventions::api_routes::plugin_settings(plugin_id);
    match qol_plugin_api::host_exec::post_to_daemon(&path, "") {
        Ok((status, _)) if (200..300).contains(&status) => Ok(()),
        Ok((status, _)) => Err(io::Error::other(format!(
            "qol-tray refused to open {plugin_id} settings (HTTP {status})"
        ))),
        Err(error) => Err(io::Error::other(format!(
            "qol-tray did not answer the {plugin_id} settings request: {error}"
        ))),
    }
}

/// Reveals an existing path in the platform file manager without blocking the
/// caller on the file manager process.
pub fn reveal_in_file_manager(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("path does not exist: {}", path.display()),
        ));
    }

    match platform::reveal_plan(path) {
        RevealPlan::Open(target) => open_with_default_app(target),
        RevealPlan::Command(mut command) => {
            prepare_external_command(&mut command);
            qol_process::spawn_detached(&mut command)
        }
    }
}

fn prepare_external_command(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove(qol_conventions::ENV_DAEMON_SOCKET)
        .env_remove(qol_conventions::ENV_INSTALL_ID);
    if let Some(dir) = qol_platform::launch_working_dir() {
        command.current_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_rejects_a_missing_path_before_launching() {
        let temp = tempfile::tempdir().unwrap();
        let error = reveal_in_file_manager(&temp.path().join("missing")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
