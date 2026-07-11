#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

pub(super) fn discover_installed_apps() -> Vec<qol_apps::InstalledApp> {
    #[cfg(target_os = "macos")]
    return macos::discover_installed_apps();

    #[cfg(not(target_os = "macos"))]
    fallback::discover_installed_apps()
}
