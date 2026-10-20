use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Result};
use qol_process::{run_owned_with_output_timeout, BoundedCommandOutput};

use crate::lock::Availability;

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 4096;
const INSTALL_FIX: &str =
    "Install cinnamon-screensaver, xdg-utils or systemd-logind with a screen locker.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Locker {
    program: &'static str,
    args: &'static [&'static str],
}

const LOCKERS: [Locker; 3] = [
    Locker {
        program: "cinnamon-screensaver-command",
        args: &["--lock"],
    },
    Locker {
        program: "xdg-screensaver",
        args: &["lock"],
    },
    Locker {
        program: "loginctl",
        args: &["lock-session"],
    },
];

pub fn lock_screen() -> Result<()> {
    let lockers = installed(on_path);
    if lockers.is_empty() {
        bail!("no screen locker found. {INSTALL_FIX}");
    }
    let mut failures = Vec::new();
    for locker in lockers {
        match run(locker) {
            Ok(()) => return Ok(()),
            Err(error) => failures.push(format!("{}: {error}", locker.program)),
        }
    }
    bail!("every screen locker failed: {}", failures.join("; "))
}

pub fn availability() -> Availability {
    match installed(on_path).first() {
        Some(locker) => Availability::Ready(format!("{} will lock the screen.", locker.program)),
        None => Availability::Missing {
            detail: "No screen locker is on PATH.".into(),
            fix: INSTALL_FIX.into(),
        },
    }
}

fn installed(is_installed: impl Fn(&str) -> bool) -> Vec<Locker> {
    LOCKERS
        .into_iter()
        .filter(|locker| is_installed(locker.program))
        .collect()
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| is_file(&directory.join(program)))
    })
}

fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

fn run(locker: Locker) -> Result<()> {
    let mut command = Command::new(locker.program);
    command.args(locker.args);
    match run_owned_with_output_timeout(command, LOCK_TIMEOUT, OUTPUT_LIMIT)? {
        BoundedCommandOutput::Completed(output) if output.status.success() => Ok(()),
        BoundedCommandOutput::Completed(output) => bail!(
            "exited with {}: {}",
            output.status,
            String::from_utf8_lossy(output.stderr.as_bytes()).trim()
        ),
        BoundedCommandOutput::TimedOut { .. } => {
            bail!("timed out after {}s", LOCK_TIMEOUT.as_secs())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_lockers_keep_the_desktop_first_order() {
        let cases: [(&[&str], &[&str]); 4] = [
            (&[], &[]),
            (&["loginctl"], &["loginctl"]),
            (
                &["loginctl", "xdg-screensaver"],
                &["xdg-screensaver", "loginctl"],
            ),
            (
                &[
                    "loginctl",
                    "cinnamon-screensaver-command",
                    "xdg-screensaver",
                ],
                &[
                    "cinnamon-screensaver-command",
                    "xdg-screensaver",
                    "loginctl",
                ],
            ),
        ];

        for (present, expected) in cases {
            let programs: Vec<&str> = installed(|program| present.contains(&program))
                .into_iter()
                .map(|locker| locker.program)
                .collect();
            assert_eq!(programs, expected, "present: {present:?}");
        }
    }
}
