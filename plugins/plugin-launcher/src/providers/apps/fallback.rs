#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use super::{AppEntry, AppsProvider};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub struct FallbackAppsProvider;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl AppsProvider for FallbackAppsProvider {
    fn load_entries(&self) -> Vec<AppEntry> {
        Vec::new()
    }
}
