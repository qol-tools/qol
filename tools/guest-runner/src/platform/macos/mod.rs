use super::GuestRunnerPlatform;
use crate::cli::RunOptions;
use anyhow::{bail, Result};
use qol_headless::DoctorCheckResult;
use std::path::PathBuf;

pub(crate) struct Platform;

impl GuestRunnerPlatform for Platform {
    fn default_options(&self) -> RunOptions {
        RunOptions {
            device_path: PathBuf::new(),
            identity_path: PathBuf::new(),
            run_id_path: None,
        }
    }

    fn run(&self, _options: RunOptions) -> Result<()> {
        bail!("qol-guest-runner is only supported inside Linux and Windows guests")
    }

    fn platform_check(&self) -> DoctorCheckResult {
        DoctorCheckResult::fail(
            "platform_supported",
            "qol-guest-runner is only supported inside Linux and Windows guests",
        )
    }

    fn runtime_paths_check(&self) -> DoctorCheckResult {
        DoctorCheckResult::warn(
            "runtime_paths",
            "guest-control paths are not inspected on unsupported platforms",
        )
    }
}
