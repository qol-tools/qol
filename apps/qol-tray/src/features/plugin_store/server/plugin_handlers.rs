use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;

use super::helpers::{validate_plugin_id, validate_plugin_id_bad_request};
use super::plugin_services;
use super::types::{
    AppState, EnsurePluginCapabilitiesResult, ExecuteActionResult, InstalledPluginsResponse,
    PluginCapabilitiesStatus, PluginsQuery, PluginsResponse, UninstallResult,
};
use crate::plugins::action_executor::ActionExecutionError;
use crate::plugins::manifest::Capabilities;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/plugins", get(list_plugins))
        .route("/installed", get(list_installed))
        .route("/events", get(sse_handler))
        .route("/plugins/{id}/capabilities", get(get_plugin_capabilities))
        .route(
            "/plugins/{id}/capabilities/ensure",
            post(ensure_plugin_capabilities),
        )
        .route(
            "/plugins/{id}/actions/{action}",
            post(execute_plugin_action),
        )
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

pub(super) async fn get_plugin_capabilities(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PluginCapabilitiesStatus>, StatusCode> {
    if validate_plugin_id(&id).is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let manager = state
        .plugin_manager
        .lock()
        .map_err(plugin_manager_lock_failed)?;
    let plugin = manager.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    let results =
        crate::plugins::capabilities::check_capabilities(&[&plugin.manifest.capabilities]);
    Ok(Json(capability_status(
        &plugin.manifest.capabilities,
        &results,
    )))
}

pub(super) async fn ensure_plugin_capabilities(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> (StatusCode, Json<EnsurePluginCapabilitiesResult>) {
    if validate_plugin_id(&id).is_err() {
        return capability_response(
            StatusCode::BAD_REQUEST,
            false,
            "Invalid plugin ID".to_string(),
            PluginCapabilitiesStatus::default(),
        );
    }

    let mut manager = match state.plugin_manager.lock() {
        Ok(manager) => manager,
        Err(error) => {
            log::error!("Plugin manager mutex poisoned: {}", error);
            return capability_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                false,
                "Plugin manager unavailable".to_string(),
                PluginCapabilitiesStatus::default(),
            );
        }
    };
    let capabilities = match manager.get(&id) {
        Some(plugin) => plugin.manifest.capabilities.clone(),
        None => {
            return capability_response(
                StatusCode::NOT_FOUND,
                false,
                "Plugin not found".to_string(),
                PluginCapabilitiesStatus::default(),
            );
        }
    };
    let results = crate::plugins::capabilities::ensure_capabilities(&[&capabilities]);
    let status = capability_status(&capabilities, &results);
    if !status.met {
        return capability_response(
            StatusCode::OK,
            false,
            "Permissions were not granted".to_string(),
            status,
        );
    }
    if let Err(error) = manager.ensure_plugin_daemon_running(&id) {
        log::error!(
            "Failed to start plugin daemon after capability grant for {}: {}",
            id,
            error
        );
        return capability_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            false,
            "Permissions granted, but plugin startup failed".to_string(),
            status,
        );
    }

    capability_response(
        StatusCode::OK,
        true,
        "Permissions granted".to_string(),
        status,
    )
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

fn capability_status(
    capabilities: &Capabilities,
    results: &HashMap<&'static str, bool>,
) -> PluginCapabilitiesStatus {
    PluginCapabilitiesStatus {
        met: crate::plugins::capabilities::capabilities_met(capabilities, results),
        required: crate::plugins::capabilities::required_capability_names(capabilities)
            .into_iter()
            .map(str::to_string)
            .collect(),
        unmet: crate::plugins::capabilities::unmet_capability_names(capabilities, results)
            .into_iter()
            .map(str::to_string)
            .collect(),
    }
}

fn capability_response(
    status: StatusCode,
    success: bool,
    message: String,
    capability_status: PluginCapabilitiesStatus,
) -> (StatusCode, Json<EnsurePluginCapabilitiesResult>) {
    (
        status,
        Json(EnsurePluginCapabilitiesResult {
            success,
            message,
            status: capability_status,
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
