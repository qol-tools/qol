use axum::Json;
use serde_json::Value;

mod platform;

pub(in super::super) async fn list_apps() -> Json<Vec<Value>> {
    let apps = tokio::task::spawn_blocking(platform::discover_installed_apps)
        .await
        .unwrap_or_default();
    Json(apps)
}
