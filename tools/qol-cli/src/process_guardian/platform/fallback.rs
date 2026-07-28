use std::path::PathBuf;

use anyhow::{Context, Result};

use super::GuardianPlatform;

pub(super) struct Platform;

impl GuardianPlatform for Platform {
    fn guardian_executable(&self) -> Result<PathBuf> {
        std::env::current_exe().context("failed to locate the qol executable")
    }
}
