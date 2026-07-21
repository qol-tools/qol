use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::core::guards::{CaskIndex, CaskToken};
use crate::core::{AppPlatform, Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

#[derive(Default)]
pub struct Platform;

impl Platform {
    pub fn new() -> Self {
        Self
    }
}

const UNSUPPORTED: &str = "removeapp: not implemented on this platform yet";

pub(crate) fn metadata_identity(meta: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    use std::os::unix::fs::MetadataExt;
    (Some(meta.dev()), Some(meta.ino()))
}

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
        Err(anyhow!(UNSUPPORTED))
    }
    fn scan(&self, _app: &InstalledApp, _inventory: &[InstalledApp]) -> Result<RemovalPlan> {
        Err(anyhow!(UNSUPPORTED))
    }
    fn remove_items(&self, _items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome> {
        Err(anyhow!(UNSUPPORTED))
    }
    fn is_protected(&self, _app: &InstalledApp) -> bool {
        true
    }
    fn is_running(&self, _app: &InstalledApp) -> bool {
        false
    }
    fn quit(&self, _app: &InstalledApp) -> Result<()> {
        Err(anyhow!(UNSUPPORTED))
    }
    fn cask_index(&self) -> CaskIndex {
        CaskIndex::absent()
    }
    fn brew_uninstall(&self, _token: &CaskToken) -> Result<()> {
        Err(anyhow!(UNSUPPORTED))
    }
}
