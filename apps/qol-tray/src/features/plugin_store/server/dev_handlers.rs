#![cfg(feature = "dev")]

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::dev;
use crate::dev::state::DiscoveryStatus;
use crate::paths::is_safe_path_component;

use std::path::PathBuf;

use super::dev_services;
use super::types::{
    AppState, BuildStateResponse, DiscoveryStateResponse, MockTargetInfo,
    UpsertPluginLogControlRequest,
};

fn config_dir() -> Result<PathBuf, (StatusCode, String)> {
    crate::paths::shared_config_dir().map_err(|e| {
        log::error!("Failed to determine config directory: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Config dir unavailable".to_string(),
        )
    })
}

pub(super) async fn reload_plugins(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(message) = dev_services::queue_reload(&state) {
        log::warn!("Developer reload requested, but a build is already in progress");
        return (StatusCode::CONFLICT, message).into_response();
    }
    (StatusCode::OK, "Reload queued").into_response()
}

pub(super) async fn recompile_self(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(message) = dev_services::queue_self_recompile(&state) {
        return (StatusCode::CONFLICT, message).into_response();
    }
    StatusCode::ACCEPTED.into_response()
}

pub(super) async fn list_linked_plugins(
    State(_state): State<AppState>,
) -> Result<Json<Vec<dev::LinkedPlugin>>, StatusCode> {
    let config_dir = crate::paths::shared_config_dir().map_err(|e| {
        log::error!("Failed to determine config directory: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    tokio::task::spawn_blocking(move || dev::list_linked_plugins(&config_dir))
        .await
        .map_err(|e| {
            log::error!("Linked plugin listing worker failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .map_err(|e| {
            log::error!("Failed to list linked plugins: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub(super) async fn create_link(
    State(state): State<AppState>,
    Json(req): Json<dev::LinkRequest>,
) -> impl IntoResponse {
    let config_dir = match config_dir() {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };
    let source = std::path::Path::new(&req.path);

    match dev::create_link(source, &config_dir) {
        Ok(_) => {
            crate::dev::state::start_discovery(
                &state.dev_state,
                &state.daemon.events,
                state.plugins_dir.clone(),
            );
            (StatusCode::OK, "Link created".to_string()).into_response()
        }
        Err(e) if e.contains("Already linked") => (StatusCode::CONFLICT, e).into_response(),
        Err(e) if e.contains("does not exist") || e.contains("No plugin.toml") => {
            (StatusCode::BAD_REQUEST, e).into_response()
        }
        Err(e) => {
            log::error!("Failed to create link: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}

pub(super) async fn delete_link(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_safe_path_component(&id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string()).into_response();
    }

    let config_dir = match config_dir() {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    match dev::remove_link(&id, &config_dir) {
        Ok(()) => {
            crate::dev::state::start_discovery(
                &state.dev_state,
                &state.daemon.events,
                state.plugins_dir.clone(),
            );
            (StatusCode::OK, "Unlinked".to_string()).into_response()
        }
        Err(e) => {
            log::error!("Failed to remove link for {}: {}", id, e);
            (StatusCode::BAD_REQUEST, e).into_response()
        }
    }
}

pub(super) async fn upsert_plugin_log_control(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<UpsertPluginLogControlRequest>,
) -> impl IntoResponse {
    if !is_safe_path_component(&id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string()).into_response();
    }

    let config_dir = match config_dir() {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    let control = crate::plugins::log_control::PluginLogControl {
        muted: req.muted,
        suppress_patterns: req.suppress_patterns,
    };

    match crate::plugins::log_control::upsert_control(&config_dir, &id, control) {
        Ok(()) => {
            if let Ok(mut manager) = state.plugin_manager.lock() {
                if let Err(error) = manager.restart_running_plugin_daemon(&id) {
                    log::warn!(
                        "Updated log control for {}, but failed to restart running daemon: {}",
                        id,
                        error
                    );
                }
            }
            (StatusCode::OK, "Updated".to_string()).into_response()
        }
        Err(e) => {
            log::error!("Failed to upsert log control for {}: {}", id, e);
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}

pub(super) async fn get_discovery_state(
    State(state): State<AppState>,
) -> Json<DiscoveryStateResponse> {
    let guard = match state.dev_state.discovery.read() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!("Discovery state lock poisoned: {}", error);
            return Json(DiscoveryStateResponse {
                status: "idle".to_string(),
                plugins: Vec::new(),
            });
        }
    };
    let status = match guard.status {
        DiscoveryStatus::Idle => "idle",
        DiscoveryStatus::Discovering => "discovering",
        DiscoveryStatus::Complete => "complete",
    };
    Json(DiscoveryStateResponse {
        status: status.to_string(),
        plugins: guard.plugins.clone(),
    })
}

pub(super) async fn get_build_state(State(state): State<AppState>) -> Json<BuildStateResponse> {
    Json(state.runtime.build_state_snapshot())
}

pub(super) async fn get_plugin_cpu(
    State(state): State<AppState>,
) -> Json<super::dev_plugin_cpu::PluginCpuResponse> {
    Json(state.plugin_cpu.snapshot())
}

pub(super) async fn get_log_controls(
) -> Json<std::collections::HashMap<String, crate::plugins::log_control::PluginLogControl>> {
    let controls = crate::paths::shared_config_dir()
        .ok()
        .map(|dir| crate::plugins::log_control::load_all_controls(&dir))
        .unwrap_or_default();
    Json(controls)
}

pub(super) async fn trigger_discovery(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Discovery refresh requested");
    crate::dev::state::start_discovery(
        &state.dev_state,
        &state.daemon.events,
        state.plugins_dir.clone(),
    );
    StatusCode::OK
}

pub(super) async fn mock_check_update() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": true, "latest": "99.0.0" }))
}

pub(super) async fn list_mock_targets(State(state): State<AppState>) -> Json<Vec<MockTargetInfo>> {
    Json(state.runtime.list_mock_targets())
}

pub(super) async fn start_mock_targets(State(state): State<AppState>) -> impl IntoResponse {
    let started = match dev_services::start_mock_targets(&state) {
        Ok(started) => started,
        Err(message) => return (StatusCode::CONFLICT, message).into_response(),
    };
    mock_targets_response(
        StatusCode::ACCEPTED,
        "started",
        started,
        state.runtime.list_mock_targets(),
    )
}

pub(super) async fn stop_mock_targets(State(state): State<AppState>) -> impl IntoResponse {
    let stopped = dev_services::stop_mock_targets(&state);

    let status = if stopped.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };

    mock_targets_response(
        status,
        "stopped",
        stopped,
        state.runtime.list_mock_targets(),
    )
}

pub(super) async fn mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        state
            .runtime
            .start_mock_self_update(state.daemon.events.clone()),
        "Mock update queued",
    )
}

pub(super) async fn stop_mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_self_update(),
        "Stopping mock update",
        "No mock update in progress",
    )
}

pub(super) async fn mock_self_recompile(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        state
            .runtime
            .start_mock_self_recompile(state.daemon.events.clone()),
        "Mock recompile queued",
    )
}

pub(super) async fn stop_mock_self_recompile(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_self_recompile(),
        "Stopping mock recompile",
        "No mock recompile in progress",
    )
}

pub(super) async fn stop_mock_plugin_build(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_plugin_build(),
        "Stopping mock build",
        "No mock build in progress",
    )
}

pub(super) async fn mock_plugin_build(State(state): State<AppState>) -> impl IntoResponse {
    let events = state.daemon.events.clone();
    let config_dir = crate::paths::shared_config_dir().ok();
    mock_start_response(
        state
            .runtime
            .start_mock_plugin_build(events, config_dir, fallback_plugin_ids(&state)),
        "Mock build queued",
    )
}

pub(super) fn fallback_plugin_ids(state: &AppState) -> Vec<String> {
    state
        .dev_state
        .discovery
        .read()
        .map(|discovery| {
            discovery
                .plugins
                .iter()
                .map(|plugin| plugin.id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn mock_start_response(
    result: Result<(), &'static str>,
    queued_message: &'static str,
) -> axum::response::Response {
    match result {
        Ok(()) => (StatusCode::ACCEPTED, queued_message).into_response(),
        Err(message) => (StatusCode::CONFLICT, message).into_response(),
    }
}

fn mock_stop_response(
    stopped: bool,
    stopping_message: &'static str,
    idle_message: &'static str,
) -> axum::response::Response {
    if stopped {
        return (StatusCode::ACCEPTED, stopping_message).into_response();
    }
    (StatusCode::OK, idle_message).into_response()
}

fn mock_targets_response(
    status: StatusCode,
    key: &'static str,
    ids: Vec<&'static str>,
    targets: Vec<MockTargetInfo>,
) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({ key: ids, "targets": targets })),
    )
        .into_response()
}
