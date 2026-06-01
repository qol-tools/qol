#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

use serde_json::Value;

pub(super) fn discover_installed_apps() -> Vec<Value> {
    #[cfg(target_os = "macos")]
    return macos::discover_installed_apps();

    #[cfg(not(target_os = "macos"))]
    fallback::discover_installed_apps()
}
