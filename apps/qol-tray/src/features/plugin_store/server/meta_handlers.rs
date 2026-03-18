use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/enabled", get(dev_enabled))
        .route("/version", get(get_version))
        .route("/check-update", get(check_update))
        .route("/self-update", post(self_update))
}

pub(super) async fn dev_enabled() -> Json<bool> {
    Json(cfg!(feature = "dev"))
}

pub(super) async fn get_version() -> &'static str {
    #[cfg(feature = "dev")]
    if let Some(v) = crate::version::test_version_override() {
        return v;
    }
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
