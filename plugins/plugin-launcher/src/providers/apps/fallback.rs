#[cfg(not(target_os = "linux"))]
use super::{AppEntry, AppsProvider};

#[cfg(not(target_os = "linux"))]
pub struct FallbackAppsProvider;

#[cfg(not(target_os = "linux"))]
impl AppsProvider for FallbackAppsProvider {
    fn load_entries(&self) -> Vec<AppEntry> {
        Vec::new()
    }
}
