use anyhow::{bail, Result};

use super::PlatformOps;
use crate::installer::uninstall::model::{ProcessTargets, UninstallContext};

pub(in crate::installer::uninstall) struct Platform;

impl PlatformOps for Platform {
    fn context(&self) -> Result<UninstallContext> {
        bail!("verified uninstall is currently supported on Linux")
    }

    fn managed_processes(&self) -> Vec<crate::plugins::daemon_tracker::ManagedProcess> {
        Vec::new()
    }

    fn stop_processes(&self, _targets: &ProcessTargets) -> Result<()> {
        bail!("verified uninstall is currently supported on Linux")
    }

    fn refresh_desktop_caches(&self, _context: &UninstallContext) -> Result<()> {
        bail!("verified uninstall is currently supported on Linux")
    }
}
