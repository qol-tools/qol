mod platform;

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use platform::RevealPlan;

/// Opens a path or URL with the operating system's default application without
/// blocking the caller on the launched application.
pub fn open_with_default_app(target: impl AsRef<OsStr>) -> io::Result<()> {
    open_with_default_app_using(target.as_ref(), &mut qol_process::spawn_detached)
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
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        RevealPlan::Open(target) => open_with_default_app(target),
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        RevealPlan::Command(mut command) => {
            prepare_external_command(&mut command);
            qol_process::spawn_detached(&mut command)
        }
    }
}

fn open_with_default_app_using<F>(target: &OsStr, spawn: &mut F) -> io::Result<()>
where
    F: FnMut(&mut Command) -> io::Result<()>,
{
    let mut last_error = io::Error::new(io::ErrorKind::NotFound, "no desktop opener is available");
    for mut command in open::commands(target) {
        prepare_external_command(&mut command);
        match spawn(&mut command) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
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

    fn has_removed_env(command: &Command, expected: &str) -> bool {
        command
            .get_envs()
            .any(|(key, value)| key == expected && value.is_none())
    }

    #[test]
    fn default_open_prepares_and_tries_every_launcher() {
        let expected_attempts = open::commands("https://example.invalid").len();
        let mut attempts = 0;
        let error =
            open_with_default_app_using(OsStr::new("https://example.invalid"), &mut |command| {
                attempts += 1;
                assert!(has_removed_env(command, qol_conventions::ENV_DAEMON_SOCKET));
                assert!(has_removed_env(command, qol_conventions::ENV_INSTALL_ID));
                assert_eq!(
                    command.get_current_dir(),
                    qol_platform::launch_working_dir().as_deref()
                );
                Err(io::Error::new(io::ErrorKind::NotFound, "test opener"))
            })
            .unwrap_err();

        assert_eq!(attempts, expected_attempts);
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn default_open_stops_after_the_first_success() {
        let mut attempts = 0;
        open_with_default_app_using(OsStr::new("target"), &mut |_| {
            attempts += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(attempts, 1);
    }

    #[test]
    fn reveal_rejects_a_missing_path_before_launching() {
        let temp = tempfile::tempdir().unwrap();
        let error = reveal_in_file_manager(&temp.path().join("missing")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
