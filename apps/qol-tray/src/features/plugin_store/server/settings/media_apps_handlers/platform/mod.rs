mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
use fallback as imp;
#[cfg(target_os = "windows")]
use fallback as imp;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "macos")]
use macos as imp;

trait MediaAppsPlatformOps {
    fn discover_installed_apps() -> Vec<qol_apps::InstalledApp>;
}

pub(super) fn discover_installed_apps() -> Vec<qol_apps::InstalledApp> {
    imp::Platform::discover_installed_apps()
}

const _: fallback::Platform = fallback::Platform;
