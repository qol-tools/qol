use axum::Json;
use serde_json::Value;

mod platform;
use platform::discover_installed_apps;

pub(in super::super) async fn list_apps() -> Json<Vec<Value>> {
    let apps = tokio::task::spawn_blocking(discover_installed_apps)
        .await
        .unwrap_or_default();
    Json(apps)
}
