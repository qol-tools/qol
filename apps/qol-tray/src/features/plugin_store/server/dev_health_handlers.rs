use axum::{extract::State, routing::get, Json, Router};

use crate::plugins::daemon_health::HealthSnapshot;

use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/dev/plugin-health", get(get_plugin_health))
}

async fn get_plugin_health(State(state): State<AppState>) -> Json<HealthSnapshot> {
    Json(state.daemon_health.borrow().clone())
}
