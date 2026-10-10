use std::path::PathBuf;

use crate::core::guards::{ManagedPackage, PackageIndex};
use crate::core::{Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

pub trait AppPlatform {
    fn installed_apps(&self) -> anyhow::Result<Vec<InstalledApp>>;
    fn scan(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> anyhow::Result<RemovalPlan>;
    fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> anyhow::Result<RemovalOutcome>;
    fn is_protected(&self, app: &InstalledApp) -> bool;
    fn is_running(&self, app: &InstalledApp) -> bool;
    fn quit(&self, app: &InstalledApp) -> anyhow::Result<()>;
    fn package_index(&self, inventory: &[InstalledApp]) -> PackageIndex;
    fn uninstall_package(&self, app: &InstalledApp, package: &ManagedPackage)
        -> anyhow::Result<()>;
    fn trashes_install_remnant(&self) -> bool {
        false
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod windows_paths;

#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use unsupported::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;

#[cfg(target_os = "linux")]
pub(super) use linux::metadata_identity;
#[cfg(target_os = "macos")]
pub(super) use macos::metadata_identity;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use unsupported::metadata_identity;
#[cfg(target_os = "windows")]
pub(super) use windows::metadata_identity;
