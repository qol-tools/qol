#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub(super) fn discover_installed_apps() -> Vec<serde_json::Value> {
    macos::discover_installed_apps()
}

#[cfg(not(target_os = "macos"))]
pub(super) fn discover_installed_apps() -> Vec<serde_json::Value> {
    Vec::new()
}
