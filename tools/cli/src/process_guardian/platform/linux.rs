use std::path::PathBuf;

use anyhow::Result;

use super::GuardianPlatform;

pub(super) struct Platform;

impl GuardianPlatform for Platform {
    fn guardian_executable(&self) -> Result<PathBuf> {
        Ok(PathBuf::from("/proc/self/exe"))
    }
}
