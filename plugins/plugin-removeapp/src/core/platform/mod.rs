use std::path::PathBuf;

use crate::core::guards::{CaskIndex, CaskToken};
use crate::core::{Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

pub trait AppPlatform {
    fn installed_apps(&self) -> anyhow::Result<Vec<InstalledApp>>;
    fn scan(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> anyhow::Result<RemovalPlan>;
    fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> anyhow::Result<RemovalOutcome>;
    fn is_protected(&self, app: &InstalledApp) -> bool;
    fn is_running(&self, app: &InstalledApp) -> bool;
    fn quit(&self, app: &InstalledApp) -> anyhow::Result<()>;
    fn cask_index(&self) -> CaskIndex;
    fn brew_uninstall(&self, token: &CaskToken) -> anyhow::Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;

#[cfg(target_os = "linux")]
pub(super) use linux::metadata_identity;
#[cfg(target_os = "macos")]
pub(super) use macos::metadata_identity;
#[cfg(target_os = "windows")]
pub(super) use windows::metadata_identity;
