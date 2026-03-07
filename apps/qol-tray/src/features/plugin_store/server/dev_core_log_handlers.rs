use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, put},
    Json, Router,
};

use super::helpers::shared_config_dir_or_response;
use super::types::{AppState, UpsertPluginLogControlRequest};

const VALID_SECTIONS: &[&str] = &["runtime", "plugins", "core"];

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/core-log-controls", get(get_core_log_controls))
        .route(
            "/dev/core-log-controls/{section}",
            put(upsert_core_log_control),
        )
}

async fn get_core_log_controls(
    State(state): State<AppState>,
) -> Json<std::collections::HashMap<String, crate::logging::LogControl>> {
    let controls = state
        .core_log_controls
        .read()
        .map(|c| c.clone())
        .unwrap_or_default();
    Json(controls)
}

async fn upsert_core_log_control(
    Path(section): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<UpsertPluginLogControlRequest>,
) -> impl IntoResponse {
    if !VALID_SECTIONS.contains(&section.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("Invalid section: {}. Valid: {:?}", section, VALID_SECTIONS),
        )
            .into_response();
    }

    let config_dir = match shared_config_dir_or_response("Config dir unavailable") {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    let control = crate::logging::LogControl {
        muted: req.muted,
        suppress_patterns: req.suppress_patterns,
    };

    match crate::logging::upsert_core_control(&config_dir, &section, control.clone()) {
        Ok(()) => {
            if let Ok(mut controls) = state.core_log_controls.write() {
                if control.muted || !control.suppress_patterns.is_empty() {
                    controls.insert(section, control);
                } else {
                    controls.remove(&section);
                }
            }
            (StatusCode::OK, "Updated".to_string()).into_response()
        }
        Err(e) => {
            log::error!("Failed to upsert core log control for {}: {}", section, e);
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}
