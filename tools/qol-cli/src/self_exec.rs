use std::path::Path;

use anyhow::{Context, Result};

pub(crate) const RESUME_TRAY_PID_ENV: &str = "QOL_DEV_RESUME_TRAY_PID";

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

#[cfg(unix)]
mod platform {
    use std::ffi::OsString;
    use std::os::unix::process::CommandExt;
    use std::path::Path;

    use anyhow::Result;

    pub(super) fn replace_process(binary: &Path, args: &[OsString], tray_pid: u32) -> Result<()> {
        let error = std::process::Command::new(binary)
            .args(args)
            .env(super::RESUME_TRAY_PID_ENV, tray_pid.to_string())
            .exec();
        Err(error.into())
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsString;
    use std::path::Path;

    use anyhow::{Context, Result};

    pub(super) fn replace_process(binary: &Path, args: &[OsString], tray_pid: u32) -> Result<()> {
        std::process::Command::new(binary)
            .args(args)
            .env(super::RESUME_TRAY_PID_ENV, tray_pid.to_string())
            .spawn()
            .context("failed to spawn successor qol process")?;
        std::process::exit(0);
    }
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
