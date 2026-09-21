use super::MediaAppsPlatformOps;

pub(super) struct Platform;

impl MediaAppsPlatformOps for Platform {
    fn discover_installed_apps() -> Vec<qol_apps::InstalledApp> {
        Vec::new()
    }
}
