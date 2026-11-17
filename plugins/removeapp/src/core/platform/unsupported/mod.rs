use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::core::guards::{ManagedPackage, PackageIndex};
use crate::core::{AppPlatform, Disposal, InstalledApp, RemovalOutcome, RemovalPlan};
use crate::PLUGIN_ID;

#[derive(Default)]
pub struct Platform;

impl Platform {
    pub fn new() -> Self {
        Self
    }
}

fn unsupported() -> anyhow::Error {
    anyhow!("{PLUGIN_ID}: not implemented on this platform yet")
}

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
        Err(unsupported())
    }
    fn scan(&self, _app: &InstalledApp, _inventory: &[InstalledApp]) -> Result<RemovalPlan> {
        Err(unsupported())
    }
    fn remove_items(&self, _items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome> {
        Err(unsupported())
    }
    fn is_protected(&self, _app: &InstalledApp) -> bool {
        true
    }
    fn is_running(&self, _app: &InstalledApp) -> bool {
        false
    }
    fn quit(&self, _app: &InstalledApp) -> Result<()> {
        Err(unsupported())
    }
    fn package_index(&self, _inventory: &[InstalledApp]) -> PackageIndex {
        PackageIndex::absent()
    }
    fn uninstall_package(&self, _app: &InstalledApp, _package: &ManagedPackage) -> Result<()> {
        Err(unsupported())
    }
}
