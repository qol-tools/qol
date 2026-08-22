use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::core::guards::{ManagedPackage, PackageIndex};
use crate::core::{AppPlatform, Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

#[derive(Default)]
pub struct Platform;

impl Platform {
    pub fn new() -> Self {
        Self
    }
}

const UNSUPPORTED: &str = "removeapp: not implemented on this platform yet";

pub(crate) fn metadata_identity(_meta: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    (None, None)
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
    fn package_index(&self, _inventory: &[InstalledApp]) -> PackageIndex {
        PackageIndex::absent()
    }
    fn uninstall_package(&self, _app: &InstalledApp, _package: &ManagedPackage) -> Result<()> {
        Err(anyhow!(UNSUPPORTED))
    }
}
