use axum::Json;
use serde_json::Value;

#[cfg(target_os = "macos")]
mod platform;

pub(in super::super) async fn list_apps() -> Json<Vec<Value>> {
    let apps = tokio::task::spawn_blocking(discover_installed_apps)
        .await
        .unwrap_or_default();
    Json(apps)
}

#[cfg(target_os = "macos")]
use platform::discover_installed_apps;

#[cfg(not(target_os = "macos"))]
fn discover_installed_apps() -> Vec<Value> {
    Vec::new()
}
