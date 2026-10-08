use std::env;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use qol_dev_guest::{
    GuestHello, GuestSession, DEFAULT_DEVICE_PATH, DEFAULT_IDENTITY_PATH, DEFAULT_RUN_ID_PATH,
    PROTOCOL_VERSION,
};
use qol_headless::DoctorCheckResult;

use super::GuestRunnerPlatform;
use crate::cli::RunOptions;
use qol_dev_guest::server::{read_identity, read_run_id, serve_forever};

pub(super) struct Platform;

impl GuestRunnerPlatform for Platform {
    fn default_options(&self) -> RunOptions {
        RunOptions {
            device_path: PathBuf::from(DEFAULT_DEVICE_PATH),
            identity_path: PathBuf::from(DEFAULT_IDENTITY_PATH),
            run_id_path: Some(PathBuf::from(DEFAULT_RUN_ID_PATH)),
        }
    }

    fn run(&self, options: RunOptions) -> Result<()> {
        let image = read_identity(&options.identity_path)?;
        let run_id_path = options
            .run_id_path
            .context("Linux guests read the run identity from a file")?;
        let run_id = read_run_id(&run_id_path)?;
        let hello = GuestHello {
            protocol_version: PROTOCOL_VERSION,
            run_id,
            image,
            session: current_session(),
            runner_pid: std::process::id(),
        };
        hello
            .validate_for(&hello.image.environment_id)
            .context("guest runner must start inside the prepared graphical session")?;
        serve_forever(
            || open_device(&options.device_path),
            &hello,
            |_| {},
            |error| eprintln!("qol-guest-runner: {error:#}"),
        )
    }

    fn platform_check(&self) -> DoctorCheckResult {
        DoctorCheckResult::ok(
            "platform_supported",
            "Linux guest-control runtime is supported",
        )
    }

    fn runtime_paths_check(&self) -> DoctorCheckResult {
        let options = self.default_options();
        let mut paths = vec![
            ("device", options.device_path),
            ("identity", options.identity_path),
        ];
        paths.extend(options.run_id_path.map(|path| ("run id", path)));
        let missing = paths
            .into_iter()
            .filter_map(|(label, path)| {
                (!path.exists()).then_some(format!("{label}: {}", path.display()))
            })
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return DoctorCheckResult::ok(
                "runtime_paths",
                "guest-control device and identity files are available",
            );
        }
        DoctorCheckResult::warn(
            "runtime_paths",
            format!(
                "prepared guest runtime paths are missing: {}",
                missing.join(", ")
            ),
        )
        .with_fix("run qol-guest-runner inside a prepared qol development guest")
    }
}

fn open_device(path: &Path) -> Result<(BufReader<File>, File)> {
    let device = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open guest-control device {}", path.display()))?;
    let writer = device
        .try_clone()
        .context("failed to clone guest-control device")?;
    Ok((BufReader::new(device), writer))
}

fn current_session() -> GuestSession {
    GuestSession {
        user: env::var("USER").unwrap_or_default(),
        desktop: env::var("XDG_CURRENT_DESKTOP")
            .ok()
            .and_then(|value| normalize_desktop(&value)),
        session_type: env::var("XDG_SESSION_TYPE")
            .ok()
            .map(|value| value.to_ascii_lowercase()),
        display: env::var("DISPLAY").ok().filter(|value| !value.is_empty()),
        runtime_dir: env::var("XDG_RUNTIME_DIR")
            .ok()
            .filter(|value| !value.is_empty()),
        dbus_session: env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
    }
}

fn normalize_desktop(raw: &str) -> Option<String> {
    raw.split(':')
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(|value| {
            value
                .strip_prefix("X-")
                .unwrap_or(value)
                .to_ascii_lowercase()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_cinnamon_session_names() {
        assert_eq!(
            normalize_desktop("X-Cinnamon"),
            Some("cinnamon".to_string())
        );
        assert_eq!(
            normalize_desktop("Cinnamon:GNOME"),
            Some("cinnamon".to_string())
        );
        assert_eq!(normalize_desktop(""), None);
    }
}
