use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use crate::plugins::action_executor::ActionExecutionError;
use super::helpers::{validate_plugin_id, validate_plugin_id_bad_request};
use super::plugin_services;
use super::types::{
    AppState, ExecuteActionResult, InstalledPluginsResponse, PluginsQuery, PluginsResponse,
    UninstallResult,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/plugins", get(list_plugins))
        .route("/installed", get(list_installed))
        .route("/events", get(sse_handler))
        .route("/plugins/{id}/actions/{action}", post(execute_plugin_action))
        .route("/install/{id}", post(install_plugin))
        .route("/update/{id}", post(update_plugin))
        .route("/uninstall/{id}", post(uninstall_plugin))
}

pub(super) async fn list_plugins(
    Query(query): Query<PluginsQuery>,
) -> Result<Json<PluginsResponse>, (StatusCode, String)> {
    plugin_services::list_plugins(query.refresh).await.map(Json)
}

pub(super) async fn install_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<super::types::PluginInfo>, (StatusCode, String)> {
    validate_plugin_id_bad_request(&id)?;

    plugin_services::install_plugin(&state, &id).await.map(Json)
}

pub(super) async fn update_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    if validate_plugin_id(&id).is_err() {
        return invalid_plugin_id_uninstall_result();
    }

    Json(plugin_services::update_plugin(&state, &id).await)
}

pub(super) async fn uninstall_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    if validate_plugin_id(&id).is_err() {
        return invalid_plugin_id_uninstall_result();
    }

    Json(plugin_services::uninstall_plugin(&state, &id).await)
}

pub(super) async fn execute_plugin_action(
    Path((id, action)): Path<(String, String)>,
    State(state): State<AppState>,
) -> (StatusCode, Json<ExecuteActionResult>) {
    if validate_plugin_id(&id).is_err() {
        return invalid_plugin_id_action_result();
    }
    let result =
        crate::plugins::action_executor::try_execute_action(&state.plugin_manager, &id, &action);
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(ExecuteActionResult {
                success: true,
                message: "Action dispatched".to_string(),
            }),
        ),
        Err(error) => action_error_response(&id, &action, &error),
    }
}

pub(super) async fn list_installed(
    State(state): State<AppState>,
) -> Result<Json<InstalledPluginsResponse>, StatusCode> {
    plugin_services::list_installed(&state).map(Json)
}

pub(super) async fn sse_handler(State(state): State<AppState>) -> impl IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt;

    let rx = state.daemon.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().and_then(|event| {
            serde_json::to_string(&event)
                .ok()
                .map(|json| Ok::<_, std::convert::Infallible>(Event::default().data(json)))
        })
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn invalid_plugin_id_uninstall_result() -> Json<UninstallResult> {
    Json(UninstallResult {
        success: false,
        message: "Invalid plugin ID".to_string(),
    })
}

fn invalid_plugin_id_action_result() -> (StatusCode, Json<ExecuteActionResult>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ExecuteActionResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        }),
    )
}

fn action_error_response(
    id: &str,
    action: &str,
    error: &ActionExecutionError,
) -> (StatusCode, Json<ExecuteActionResult>) {
    let status = match error {
        ActionExecutionError::PluginNotFound(_) => StatusCode::NOT_FOUND,
        ActionExecutionError::InvalidActionId(_)
        | ActionExecutionError::MissingActionMapping { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let message = if status.is_server_error() {
        log::error!("Plugin action failed for {}::{}: {}", id, action, error);
        "Action execution failed".to_string()
    } else {
        log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
        error.to_string()
    };
    (status, Json(ExecuteActionResult { success: false, message }))
}
