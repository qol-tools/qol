use super::helpers::{validate_plugin_id, validate_plugin_id_bad_request};
use super::plugin_services;
use super::types::{
    AppState, ExecuteActionResult, InstalledPluginsResponse, PluginPermissionsResponse,
    PluginsQuery, PluginsResponse, UninstallResult,
};
use crate::plugins::action_executor::ActionExecutionError;
use crate::plugins::capabilities::{PermissionState, PermissionStatus};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/plugins", get(list_plugins))
        .route("/plugins/registry", get(get_registry))
        .route("/installed", get(list_installed))
        .route("/events", get(sse_handler))
        .route("/plugins/{id}/permissions", get(get_plugin_permissions))
        .route(
            "/permissions/{name}/request",
            post(request_permission_handler),
        )
        .route(
            "/plugins/{id}/actions/{action}",
            post(execute_plugin_action),
        )
        .route("/plugins/{id}/queries/{query}", get(query_plugin_handler))
        .route("/install/{id}", post(install_plugin))
        .route("/update/{id}", post(update_plugin))
        .route("/uninstall/{id}", post(uninstall_plugin))
}

pub(super) async fn query_plugin_handler(
    Path((id, query)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_plugin_id_bad_request(&id)?;
    let runtime = crate::plugins::config::load_runable_contract(&id)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "no runable contract".to_string()))?;
    if !runtime.queries.contains_key(&query) {
        return Err((
            StatusCode::NOT_FOUND,
            format!("query not declared: {query}"),
        ));
    }
    crate::plugins::action_executor::dispatch_query(&state.plugin_manager, &id, &query)
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(super) async fn list_plugins(
    Query(query): Query<PluginsQuery>,
    State(state): State<AppState>,
) -> Result<Json<PluginsResponse>, (StatusCode, String)> {
    plugin_services::list_plugins(&state, query.refresh).map(Json)
}

pub(super) async fn get_registry(
) -> Result<Json<crate::plugins::registry::Registry>, (StatusCode, String)> {
    let config_dir = crate::paths::shared_config_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let registry = crate::plugins::registry::load_registry(&config_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(registry))
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

pub(super) async fn get_plugin_permissions(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PluginPermissionsResponse>, StatusCode> {
    if validate_plugin_id(&id).is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let manager = state
        .plugin_manager
        .lock()
        .map_err(plugin_manager_lock_failed)?;
    let plugin = manager.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    let permissions =
        crate::plugins::capabilities::check_plugin_permissions(&plugin.manifest.capabilities);
    Ok(Json(PluginPermissionsResponse { permissions }))
}

pub(super) async fn request_permission_handler(
    Path(name): Path<String>,
) -> Result<Json<PermissionStatus>, StatusCode> {
    let current =
        crate::plugins::capabilities::check_permission(&name).ok_or(StatusCode::NOT_FOUND)?;
    if current.state == PermissionState::Granted {
        return Err(StatusCode::BAD_REQUEST);
    }
    crate::plugins::capabilities::request_permission(&name)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
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

fn plugin_manager_lock_failed(
    error: std::sync::PoisonError<std::sync::MutexGuard<'_, crate::plugins::PluginManager>>,
) -> StatusCode {
    log::error!("Plugin manager mutex poisoned: {}", error);
    StatusCode::INTERNAL_SERVER_ERROR
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
        ActionExecutionError::ActionRejected(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let message = log_and_message(status, id, action, error);
    (
        status,
        Json(ExecuteActionResult {
            success: false,
            message,
        }),
    )
}

fn log_and_message(
    status: StatusCode,
    id: &str,
    action: &str,
    error: &ActionExecutionError,
) -> String {
    if status.is_server_error() {
        log::error!("Plugin action failed for {}::{}: {}", id, action, error);
        return "Action execution failed".to_string();
    }
    log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
    error.to_string()
}
