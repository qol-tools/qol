use super::GuestRunnerPlatform;
use crate::cli::RunOptions;
use anyhow::{bail, Result};
use qol_headless::DoctorCheckResult;

pub(super) struct Platform;

impl GuestRunnerPlatform for Platform {
    fn run(&self, _options: RunOptions) -> Result<()> {
        bail!("qol-guest-runner is only supported inside Linux guests")
    }

    fn platform_check(&self) -> DoctorCheckResult {
        DoctorCheckResult::fail(
            "platform_supported",
            "qol-guest-runner is only supported inside Linux guests",
        )
    }

    fn runtime_paths_check(&self) -> DoctorCheckResult {
        DoctorCheckResult::warn(
            "runtime_paths",
            "guest-control paths are not inspected on unsupported platforms",
        )
    }
}
