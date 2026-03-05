use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use super::types::AppState;

pub(super) async fn dev_enabled() -> Json<bool> {
    Json(cfg!(feature = "dev"))
}

pub(super) async fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub(super) async fn check_update() -> Json<serde_json::Value> {
    let available = crate::updates::check_for_updates().await.unwrap_or(false);
    let latest = crate::updates::latest_version().map(String::from);
    Json(serde_json::json!({ "available": available, "latest": latest }))
}

pub(super) async fn self_update(State(state): State<AppState>) -> impl IntoResponse {
    let events = state.daemon.events.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::updates::download_and_install(events.clone()).await {
            log::error!("Self-update failed: {}", e);
            events.send(crate::daemon::DaemonEvent::UpdateFailed {
                message: e.to_string(),
            });
        }
    });
    StatusCode::ACCEPTED
}
