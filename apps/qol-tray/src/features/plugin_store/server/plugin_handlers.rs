use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::paths::is_safe_path_component;

use super::plugin_services;
use super::types::{
    AppState, ExecuteActionResult, InstalledPluginsResponse, PluginsQuery, PluginsResponse,
    UninstallResult,
};

pub(super) async fn list_plugins(
    Query(query): Query<PluginsQuery>,
) -> Result<Json<PluginsResponse>, (StatusCode, String)> {
    plugin_services::list_plugins(query.refresh).await.map(Json)
}

pub(super) async fn install_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<super::types::PluginInfo>, (StatusCode, String)> {
    if !is_safe_path_component(&id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string()));
    }

    plugin_services::install_plugin(&state, &id).await.map(Json)
}

pub(super) async fn update_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    Json(plugin_services::update_plugin(&state, &id).await)
}

pub(super) async fn uninstall_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    Json(plugin_services::uninstall_plugin(&state, &id).await)
}

pub(super) async fn execute_plugin_action(
    Path((id, action)): Path<(String, String)>,
    State(state): State<AppState>,
) -> (StatusCode, Json<ExecuteActionResult>) {
    if !is_safe_path_component(&id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ExecuteActionResult {
                success: false,
                message: "Invalid plugin ID".to_string(),
            }),
        );
    }

    use crate::plugins::action_executor::ActionExecutionError;

    let result = crate::plugins::action_executor::try_execute_action(
        &state.plugin_manager,
        &id,
        &action,
    );

    let Err(error) = result else {
        return (
            StatusCode::OK,
            Json(ExecuteActionResult {
                success: true,
                message: "Action dispatched".to_string(),
            }),
        );
    };

    let (status, message) = match &error {
        ActionExecutionError::PluginNotFound(_) => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (StatusCode::NOT_FOUND, error.to_string())
        }
        ActionExecutionError::InvalidActionId(_)
        | ActionExecutionError::MissingActionMapping { .. } => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (StatusCode::BAD_REQUEST, error.to_string())
        }
        _ => {
            log::error!("Plugin action failed for {}::{}: {}", id, action, error);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Action execution failed".to_string(),
            )
        }
    };

    (
        status,
        Json(ExecuteActionResult {
            success: false,
            message,
        }),
    )
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
