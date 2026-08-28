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
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use qol_config::contract::IndexMap;

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
        .route("/plugins/{id}/settings", post(open_settings_surface))
        .route(
            "/plugins/{id}/queries/{query}",
            get(query_plugin_handler).post(query_plugin_with_input_handler),
        )
        .route("/push-status", get(get_push_status))
        .route("/install/{id}", post(install_plugin))
        .route("/update/{id}", post(update_plugin))
        .route("/uninstall/{id}", post(uninstall_plugin))
}

fn trace_action_resolve(
    phase: &str,
    plugin_id: &str,
    action_id: &str,
    #[cfg(debug_assertions)] started: &std::time::Instant,
    #[cfg(not(debug_assertions))] started: &(),
) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "ACTION_RESOLVE",
        "plugin={plugin_id} action={action_id} phase={phase} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    #[cfg(not(debug_assertions))]
    let _ = (phase, plugin_id, action_id, started);
}

fn load_query_contract(
    plugin_id: &str,
    query: &str,
) -> Result<qol_config::contract::RuntimeSpec, (StatusCode, String)> {
    let runtime = crate::plugins::config::load_runable_contract(plugin_id)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "no runable contract".to_string()))?;
    if !runtime.queries.contains_key(query) {
        return Err((
            StatusCode::NOT_FOUND,
            format!("query not declared: {query}"),
        ));
    }
    Ok(runtime)
}

pub(super) async fn query_plugin_handler(
    Path((id, query)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_plugin_id_bad_request(&id)?;
    tokio::task::spawn_blocking(move || {
        load_query_contract(&id, &query)?;
        crate::plugins::action_executor::dispatch_query(&state.plugin_manager, &id, &query)
            .map(Json)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
    })
    .await
    .map_err(join_error_response)?
}

pub(super) async fn query_plugin_with_input_handler(
    Path((id, query)): Path<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_plugin_id_bad_request(&id)?;
    let agent_home = crate::features::agents::header_agent_home(&headers);
    tokio::task::spawn_blocking(move || {
        let runtime = load_query_contract(&id, &query)?;
        let input = amend_input_with_agent_home(
            input,
            accepts_agent_home(
                runtime
                    .queries
                    .get(&query)
                    .and_then(|entry| entry.input.as_ref()),
            ),
            agent_home,
        );
        crate::plugins::action_executor::dispatch_query_with_input(
            &state.plugin_manager,
            &id,
            &query,
            input,
            crate::plugins::action_executor::MCP_DISPATCH_TIMEOUT,
        )
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
    })
    .await
    .map_err(join_error_response)?
}

fn amend_input_with_agent_home(
    mut input: serde_json::Value,
    accepts: bool,
    agent_home: Option<String>,
) -> serde_json::Value {
    if let (true, Some(agent_home)) = (accepts, agent_home) {
        if let Some(map) = input.as_object_mut() {
            map.insert(
                "agent_home".to_owned(),
                serde_json::Value::String(agent_home),
            );
        }
    }
    input
}

fn accepts_agent_home(input: Option<&IndexMap<String, String>>) -> bool {
    input.is_some_and(|map| map.contains_key("agent_home"))
}

pub(super) async fn list_plugins(
    Query(query): Query<PluginsQuery>,
    State(state): State<AppState>,
) -> Result<Json<PluginsResponse>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || plugin_services::list_plugins(&state, query.refresh))
        .await
        .map_err(join_error_response)?
        .map(Json)
}

pub(super) async fn get_registry(
) -> Result<Json<crate::plugins::registry::Registry>, (StatusCode, String)> {
    tokio::task::spawn_blocking(|| {
        let config_dir = crate::paths::shared_config_dir()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        crate::plugins::registry::load_registry(&config_dir)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
    })
    .await
    .map_err(join_error_response)?
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
    headers: HeaderMap,
    input: Option<Json<serde_json::Value>>,
) -> (StatusCode, Json<ExecuteActionResult>) {
    qol_runtime::probe!("ACTION_RECV", "plugin={} action={}", id, action);
    if validate_plugin_id(&id).is_err() {
        return invalid_plugin_id_action_result();
    }
    let plugin_manager = state.plugin_manager.clone();
    let worker_id = id.clone();
    let worker_action = action.clone();
    let worker_input = input.map(|Json(value)| value).unwrap_or_default();
    let agent_home = crate::features::agents::header_agent_home(&headers);
    #[cfg(debug_assertions)]
    let resolve_started = std::time::Instant::now();
    #[cfg(not(debug_assertions))]
    let resolve_started = ();
    match tokio::task::spawn_blocking(move || {
        trace_action_resolve("start", &worker_id, &worker_action, &resolve_started);
        let accepts = crate::plugins::config::load_runable_contract(&worker_id)
            .ok()
            .flatten()
            .map(|runtime| {
                accepts_agent_home(
                    runtime
                        .actions
                        .get(&worker_action)
                        .and_then(|entry| entry.input.as_ref()),
                )
            })
            .unwrap_or(false);
        let worker_input = amend_input_with_agent_home(worker_input, accepts, agent_home);
        let result = crate::plugins::action_executor::try_execute_action_with_input_result(
            &plugin_manager,
            &worker_id,
            &worker_action,
            worker_input,
        );
        trace_action_resolve("done", &worker_id, &worker_action, &resolve_started);
        result
    })
    .await
    {
        Ok(Ok(data)) => (
            StatusCode::OK,
            Json(ExecuteActionResult {
                success: true,
                message: "Action dispatched".to_string(),
                data,
            }),
        ),
        Ok(Err(error)) => action_error_response(&id, &action, &error),
        Err(error) => {
            log::error!(
                "Plugin action handler crashed for {}::{}: {}",
                id,
                action,
                error
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExecuteActionResult {
                    success: false,
                    message: "Handler crashed".to_string(),
                    data: None,
                }),
            )
        }
    }
}

pub(super) async fn open_settings_surface(Path(id): Path<String>) -> (StatusCode, String) {
    if validate_plugin_id(&id).is_err() {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string());
    }
    let worker_id = id.clone();
    match tokio::task::spawn_blocking(move || crate::settings_surface::request(&worker_id)).await {
        Ok(Ok(true)) => (StatusCode::OK, "Settings surface requested".to_string()),
        Ok(Ok(false)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Native settings host is unavailable on this platform".to_string(),
        ),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Native settings host failed: {error:#}"),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Settings host handler crashed: {error}"),
        ),
    }
}

pub(super) async fn list_installed(
    State(state): State<AppState>,
) -> Result<Json<InstalledPluginsResponse>, StatusCode> {
    match tokio::task::spawn_blocking(move || plugin_services::list_installed(&state)).await {
        Ok(result) => result.map(Json),
        Err(error) => {
            log::error!("list_installed join error: {}", error);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Latest pushed status per plugin id (empty when no plugin pushed one yet).
pub(super) async fn get_push_status() -> Json<std::collections::HashMap<String, serde_json::Value>>
{
    Json(crate::runtime::PluginStatusRegistry::shared().snapshot())
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
            data: None,
        }),
    )
}

fn plugin_manager_lock_failed(
    error: std::sync::PoisonError<std::sync::MutexGuard<'_, crate::plugins::PluginManager>>,
) -> StatusCode {
    log::error!("Plugin manager mutex poisoned: {}", error);
    StatusCode::INTERNAL_SERVER_ERROR
}

fn join_error_response(error: tokio::task::JoinError) -> (StatusCode, String) {
    log::error!("plugin handler join error: {}", error);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Handler crashed".to_string(),
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
        ActionExecutionError::ActionRejected(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let message = log_and_message(status, id, action, error);
    (
        status,
        Json(ExecuteActionResult {
            success: false,
            message,
            data: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    const RUNTIME_WITH_AGENT_HOME: &str = r#"
schema_version = 1

[query.do_it]
description = "does it"
poll_interval_ms = 1000
input = { question = "Question", agent_home = "Agent home id" }

[action.do_that]
description = "does it"
input = { question = "Question", agent_home = "Agent home id" }
"#;

    #[test]
    fn query_input_copies_agent_home_only_when_declared_and_header_present() {
        let runtime =
            qol_config::contract::parse_runtime_spec_str(RUNTIME_WITH_AGENT_HOME).unwrap();
        let accepts = |query: &str| {
            accepts_agent_home(
                runtime
                    .queries
                    .get(query)
                    .and_then(|entry| entry.input.as_ref()),
            )
        };
        let amended = amend_input_with_agent_home(
            serde_json::json!({"question": "q"}),
            accepts("do_it"),
            Some("/home/k/.claude-work".to_owned()),
        );
        assert_eq!(amended["question"], "q");
        assert_eq!(amended["agent_home"], "/home/k/.claude-work");
        let undeclared = amend_input_with_agent_home(
            serde_json::json!({"question": "q", "agent_home": "caller-chosen"}),
            accepts("missing"),
            Some("/home/k/.claude-work".to_owned()),
        );
        assert_eq!(
            undeclared,
            serde_json::json!({"question": "q", "agent_home": "caller-chosen"})
        );
        let headerless = amend_input_with_agent_home(
            serde_json::json!({"question": "q"}),
            accepts("do_it"),
            None,
        );
        assert_eq!(headerless, serde_json::json!({"question": "q"}));
    }

    #[test]
    fn action_input_copies_agent_home_only_when_declared_and_header_present() {
        let runtime =
            qol_config::contract::parse_runtime_spec_str(RUNTIME_WITH_AGENT_HOME).unwrap();
        let accepts = |action: &str| {
            accepts_agent_home(
                runtime
                    .actions
                    .get(action)
                    .and_then(|entry| entry.input.as_ref()),
            )
        };
        let amended = amend_input_with_agent_home(
            serde_json::json!({"question": "q"}),
            accepts("do_that"),
            Some("/home/k/.claude-work".to_owned()),
        );
        assert_eq!(amended["question"], "q");
        assert_eq!(amended["agent_home"], "/home/k/.claude-work");
        let undeclared = amend_input_with_agent_home(
            serde_json::json!({"question": "q"}),
            accepts("missing"),
            Some("/home/k/.claude-work".to_owned()),
        );
        assert_eq!(undeclared, serde_json::json!({"question": "q"}));
        let headerless = amend_input_with_agent_home(
            serde_json::json!({"question": "q"}),
            accepts("do_it"),
            None,
        );
        assert_eq!(headerless, serde_json::json!({"question": "q"}));
    }
}
