use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::core::{AppPlatform, Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

#[derive(Default)]
pub struct Platform;

const UNSUPPORTED: &str = "removeapp: not implemented on this platform yet";

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
        Err(anyhow!(UNSUPPORTED))
    }
    fn scan(&self, _app: &InstalledApp) -> Result<RemovalPlan> {
        Err(anyhow!(UNSUPPORTED))
    }
    fn remove_paths(&self, _paths: &[PathBuf], _how: Disposal) -> Result<RemovalOutcome> {
        Err(anyhow!(UNSUPPORTED))
    }
    fn is_protected(&self, _app: &InstalledApp) -> bool {
        true
    }
}
