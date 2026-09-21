use std::path::Path;

use anyhow::{Context, Result};

pub(crate) const RESUME_TRAY_PID_ENV: &str = "QOL_DEV_RESUME_TRAY_PID";
#[cfg(unix)]
pub(crate) const PRIOR_TERMIOS_ENV: &str = "QOL_DEV_PRIOR_TERMIOS";

mod platform;

#[cfg(unix)]
mod termios;
#[cfg(unix)]
pub(crate) use termios::{apply_prior_termios, capture_prior_termios, restore_resumed_tty};

#[cfg(not(unix))]
pub(crate) fn apply_prior_termios() {}
#[cfg(not(unix))]
pub(crate) fn capture_prior_termios() {}
#[cfg(not(unix))]
pub(crate) fn restore_resumed_tty() {}

pub(crate) fn resume_tray_pid() -> Option<u32> {
    std::env::var(RESUME_TRAY_PID_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
}

pub(crate) fn replace_with(binary: &Path, tray_pid: u32) -> Result<()> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    platform::replace_process(binary, &args, tray_pid)
        .with_context(|| format!("failed to relaunch {}", binary.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_tray_pid_parses_or_returns_none() {
        let cases = [
            (Some("55233"), Some(55233)),
            (Some("not-a-pid"), None),
            (Some(""), None),
            (None, None),
        ];
        for (raw, expected) in cases {
            match raw {
                Some(value) => std::env::set_var(RESUME_TRAY_PID_ENV, value),
                None => std::env::remove_var(RESUME_TRAY_PID_ENV),
            }
            assert_eq!(resume_tray_pid(), expected, "input: {raw:?}");
        }
        std::env::remove_var(RESUME_TRAY_PID_ENV);
    }
}
