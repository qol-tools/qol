mod platform;

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use platform::RevealPlan;

const CHECKED_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens a path or URL with the operating system's default application without
/// blocking the caller on the launched application.
pub fn open_with_default_app(target: impl AsRef<OsStr>) -> io::Result<()> {
    open_with_default_app_using(target.as_ref(), &mut qol_process::spawn_detached)
}

pub fn open_with_default_app_checked(target: impl AsRef<OsStr>) -> io::Result<()> {
    open_with_default_app_checked_using(
        target.as_ref(),
        &mut |command| command.spawn(),
        &mut wait_for_checked_opener,
    )
}

/// Asks the running tray to focus this plugin in the unified settings window,
/// falling back to the plugin's web settings page when no daemon answers.
pub fn open_plugin_settings(plugin_id: &str) -> io::Result<()> {
    let path = qol_conventions::api_routes::plugin_settings(plugin_id);
    if let Ok((status, _)) = qol_plugin_api::host_exec::post_to_daemon(&path, "") {
        if (200..300).contains(&status) {
            return Ok(());
        }
    }
    open_with_default_app(qol_conventions::settings_url(plugin_id))
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

fn open_with_default_app_checked_using<T, Spawn, Wait>(
    target: &OsStr,
    spawn: &mut Spawn,
    wait: &mut Wait,
) -> io::Result<()>
where
    Spawn: FnMut(&mut Command) -> io::Result<T>,
    Wait: FnMut(&mut T, Duration) -> io::Result<()>,
{
    open_with_default_app_checked_commands(open::commands(target), spawn, wait)
}

fn open_with_default_app_checked_commands<T, Spawn, Wait>(
    commands: impl IntoIterator<Item = Command>,
    spawn: &mut Spawn,
    wait: &mut Wait,
) -> io::Result<()>
where
    Spawn: FnMut(&mut Command) -> io::Result<T>,
    Wait: FnMut(&mut T, Duration) -> io::Result<()>,
{
    let mut last_error = io::Error::new(io::ErrorKind::NotFound, "no desktop opener is available");
    let mut first_wait_error = None;
    for mut command in commands {
        prepare_external_command(&mut command);
        match spawn(&mut command) {
            Ok(mut child) => match wait(&mut child, CHECKED_OPEN_TIMEOUT) {
                Ok(()) => return Ok(()),
                Err(error) if first_wait_error.is_none() => first_wait_error = Some(error),
                Err(_) => {}
            },
            Err(error) => last_error = error,
        }
    }
    if let Some(error) = first_wait_error {
        return Err(error);
    }
    Err(last_error)
}

fn wait_for_checked_opener(child: &mut Child, timeout: Duration) -> io::Result<()> {
    let status = qol_process::wait_for_exit_or_terminate(child, timeout)?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "desktop opener exited with {status}"
        )))
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

    fn test_openers(count: usize) -> Vec<Command> {
        (0..count)
            .map(|index| Command::new(format!("test-opener-{index}")))
            .collect()
    }

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
    fn checked_open_stops_after_the_first_successful_exit() {
        let mut spawned = 0;
        let mut waited = 0;
        open_with_default_app_checked_commands(
            test_openers(2),
            &mut |_| {
                spawned += 1;
                Ok(())
            },
            &mut |_, timeout| {
                waited += 1;
                assert_eq!(timeout, CHECKED_OPEN_TIMEOUT);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(spawned, 1);
        assert_eq!(waited, 1);
    }

    #[test]
    fn checked_open_prefers_the_first_exit_error_over_spawn_errors() {
        let cases = [
            (
                "exit then spawn",
                [Ok("exit status 4"), Err("spawn error 2")],
                "exit status 4",
            ),
            (
                "spawn then exit",
                [Err("spawn error 1"), Ok("exit status 4")],
                "exit status 4",
            ),
            (
                "spawn then spawn",
                [Err("spawn error 1"), Err("spawn error 2")],
                "spawn error 2",
            ),
        ];

        for (name, outcomes, expected) in cases {
            let mut outcomes = outcomes.into_iter();
            let error = open_with_default_app_checked_commands(
                test_openers(2),
                &mut |_| match outcomes.next().unwrap() {
                    Ok(wait_error) => Ok(wait_error),
                    Err(spawn_error) => Err(io::Error::new(io::ErrorKind::NotFound, spawn_error)),
                },
                &mut |wait_error, _| Err(io::Error::other(*wait_error)),
            )
            .unwrap_err();

            assert_eq!(error.to_string(), expected, "{name}");
        }
    }

    #[test]
    fn checked_open_tries_the_next_candidate_after_spawn_failure() {
        let mut spawned = 0;
        let mut waited = 0;
        open_with_default_app_checked_commands(
            test_openers(2),
            &mut |_| {
                spawned += 1;
                if spawned == 1 {
                    Err(io::Error::new(io::ErrorKind::NotFound, "spawn failure"))
                } else {
                    Ok(())
                }
            },
            &mut |_, _| {
                waited += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(spawned, 2);
        assert_eq!(waited, 1);
    }

    #[test]
    fn checked_open_reports_timeout_after_each_candidate_is_killed() {
        let mut killed = 0;
        let error = open_with_default_app_checked_commands(
            test_openers(2),
            &mut |_| Ok(()),
            &mut |_, timeout| {
                assert_eq!(timeout, CHECKED_OPEN_TIMEOUT);
                killed += 1;
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out and killed",
                ))
            },
        )
        .unwrap_err();

        assert_eq!(killed, 2);
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "timed out and killed");
    }

    #[test]
    fn reveal_rejects_a_missing_path_before_launching() {
        let temp = tempfile::tempdir().unwrap();
        let error = reveal_in_file_manager(&temp.path().join("missing")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
