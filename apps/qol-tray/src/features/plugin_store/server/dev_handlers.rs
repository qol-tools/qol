#![cfg(feature = "dev")]

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::daemon::{BuildResultInfo, DaemonEvent, DiscoveryStatus};
use crate::dev;
use crate::paths::is_safe_path_component;

use super::dev_runtime::*;
use super::types::{
    AppState, BuildStateResponse, DiscoveryStateResponse, MockTargetInfo,
    UpsertPluginLogControlRequest,
};

pub(super) async fn reload_plugins(State(state): State<AppState>) -> impl IntoResponse {
    use std::sync::atomic::Ordering;

    if BUILD_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        log::warn!("Developer reload requested, but a build is already in progress");
        return (StatusCode::CONFLICT, "Build already in progress").into_response();
    }

    log::info!("Developer reload requested");

    mark_build_state_started();
    state.daemon.events.send(DaemonEvent::BuildStarted);

    let plugin_manager = state.plugin_manager.clone();
    let events = state.daemon.events.clone();
    let config_dir = crate::paths::shared_config_dir().ok();

    // Process the blocking builds asynchronously so we don't freeze the axum worker pool
    tokio::task::spawn_blocking(move || {
        struct BuildGuard;
        impl Drop for BuildGuard {
            fn drop(&mut self) {
                BUILD_IN_PROGRESS.store(false, Ordering::SeqCst);
                mark_build_state_finished();
            }
        }
        let _guard = BuildGuard;

        let dev_links = config_dir
            .as_deref()
            .map(dev::load_dev_links)
            .unwrap_or_default();
        let known_fingerprints = config_dir
            .as_deref()
            .map(dev::load_build_fingerprints)
            .unwrap_or_default();

        let build_run =
            dev::build_linked_plugins_with_progress(&dev_links, &known_fingerprints, |progress| {
                mark_build_state_progress(
                    &progress.plugin_id,
                    &progress.status,
                    progress.percent,
                    &progress.phase,
                );
                events.send(DaemonEvent::BuildPluginProgress {
                    plugin_id: progress.plugin_id,
                    status: progress.status,
                    percent: progress.percent,
                    phase: progress.phase,
                });
            });

        if let Some(config_dir) = config_dir.as_deref() {
            if let Err(e) = dev::save_build_fingerprints(config_dir, &build_run.fingerprints) {
                log::error!("Failed to persist build fingerprints: {}", e);
            }
        }

        let results: Vec<BuildResultInfo> = build_run
            .results
            .into_iter()
            .map(|r| BuildResultInfo {
                plugin_id: r.plugin_id,
                success: r.success,
                output: r.output,
                skipped: r.skipped,
            })
            .collect();

        let all_succeeded = results.is_empty() || results.iter().all(|r| r.success);
        mark_build_state_finished();
        events.send(DaemonEvent::BuildComplete { results });

        if !all_succeeded {
            return;
        }

        let mut manager = match plugin_manager.lock() {
            Ok(m) => m,
            Err(e) => {
                log::error!("Plugin manager mutex poisoned: {}", e);
                return;
            }
        };
        if let Err(e) = manager.reload_plugins() {
            log::error!("Failed to reload plugins: {}", e);
        } else {
            log::info!("Plugins reloaded successfully");
        }
    });

    (StatusCode::OK, "Reload queued").into_response()
}

pub(super) async fn recompile_self(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Developer self recompile requested");

    let events = state.daemon.events.clone();
    tokio::spawn(async move {
        let progress_events = events.clone();
        let result = tokio::task::spawn_blocking(move || {
            dev::build_qol_tray_self_with_progress(|percent, phase| {
                progress_events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
            })
        })
        .await;

        match result {
            Ok(build) if build.success => {
                events.send(DaemonEvent::SelfRecompileComplete);
            }
            Ok(build) => {
                let message = build_failure_message(&build.output);
                log::error!("Self recompile failed: {}", message);
                events.send(DaemonEvent::SelfRecompileFailed { message });
            }
            Err(e) => {
                let message = format!("Self recompile worker failed: {}", e);
                log::error!("{}", message);
                events.send(DaemonEvent::SelfRecompileFailed { message });
            }
        }
    });

    StatusCode::ACCEPTED
}

fn build_failure_message(output: &str) -> String {
    output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Self recompile failed".to_string())
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
    let config_dir = match crate::paths::shared_config_dir() {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Config dir unavailable".to_string(),
            )
                .into_response();
        }
    };
    let source = std::path::Path::new(&req.path);

    match dev::create_link(source, &config_dir) {
        Ok(_) => {
            state.daemon.start_discovery(state.plugins_dir.clone());
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

    let config_dir = match crate::paths::shared_config_dir() {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Config dir unavailable".to_string(),
            )
                .into_response();
        }
    };

    match dev::remove_link(&id, &config_dir) {
        Ok(()) => {
            state.daemon.start_discovery(state.plugins_dir.clone());
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

    let config_dir = match crate::paths::shared_config_dir() {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Config dir unavailable".to_string(),
            )
                .into_response();
        }
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
    let guard = match state.daemon.state.discovery.read() {
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

pub(super) async fn get_build_state() -> Json<BuildStateResponse> {
    Json(read_build_state_snapshot())
}

pub(super) async fn get_log_controls() -> Json<std::collections::HashMap<String, crate::plugins::log_control::PluginLogControl>> {
    let controls = crate::paths::shared_config_dir()
        .ok()
        .map(|dir| crate::plugins::log_control::load_all_controls(&dir))
        .unwrap_or_default();
    Json(controls)
}

pub(super) async fn trigger_discovery(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Discovery refresh requested");
    state.daemon.start_discovery(state.plugins_dir.clone());
    StatusCode::OK
}

pub(super) async fn mock_check_update() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": true, "latest": "99.0.0" }))
}

pub(super) async fn list_mock_targets() -> Json<Vec<MockTargetInfo>> {
    Json(mock_target_infos())
}

pub(super) async fn start_mock_targets(State(state): State<AppState>) -> impl IntoResponse {
    if any_mock_target_running() {
        return (StatusCode::CONFLICT, "Mock target already in progress").into_response();
    }

    let events = state.daemon.events.clone();
    let config_dir = crate::paths::shared_config_dir().ok();
    let mut started = Vec::new();

    if start_mock_self_update(events.clone()).is_ok() {
        started.push(super::types::MOCK_TARGET_SELF_UPDATE);
    }
    if start_mock_self_recompile(events.clone()).is_ok() {
        started.push(super::types::MOCK_TARGET_SELF_RECOMPILE);
    }
    if start_mock_plugin_build(events, config_dir).is_ok() {
        started.push(super::types::MOCK_TARGET_PLUGIN_BUILD);
    }

    if started.is_empty() {
        return (StatusCode::CONFLICT, "No mock targets were started").into_response();
    }

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "started": started, "targets": mock_target_infos() })),
    )
        .into_response()
}

pub(super) async fn stop_mock_targets() -> impl IntoResponse {
    let mut stopped = Vec::new();

    if stop_mock_self_update_internal() {
        stopped.push(super::types::MOCK_TARGET_SELF_UPDATE);
    }
    if stop_mock_self_recompile_internal() {
        stopped.push(super::types::MOCK_TARGET_SELF_RECOMPILE);
    }
    if stop_mock_plugin_build_internal() {
        stopped.push(super::types::MOCK_TARGET_PLUGIN_BUILD);
    }

    let status = if stopped.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };

    (
        status,
        Json(serde_json::json!({ "stopped": stopped, "targets": mock_target_infos() })),
    )
        .into_response()
}

pub(super) async fn mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    match start_mock_self_update(state.daemon.events.clone()) {
        Ok(()) => (StatusCode::ACCEPTED, "Mock update queued").into_response(),
        Err(msg) => (StatusCode::CONFLICT, msg).into_response(),
    }
}

pub(super) async fn stop_mock_self_update() -> impl IntoResponse {
    if stop_mock_self_update_internal() {
        (StatusCode::ACCEPTED, "Stopping mock update").into_response()
    } else {
        (StatusCode::OK, "No mock update in progress").into_response()
    }
}

pub(super) async fn mock_self_recompile(State(state): State<AppState>) -> impl IntoResponse {
    match start_mock_self_recompile(state.daemon.events.clone()) {
        Ok(()) => (StatusCode::ACCEPTED, "Mock recompile queued").into_response(),
        Err(msg) => (StatusCode::CONFLICT, msg).into_response(),
    }
}

pub(super) async fn stop_mock_self_recompile() -> impl IntoResponse {
    if stop_mock_self_recompile_internal() {
        (StatusCode::ACCEPTED, "Stopping mock recompile").into_response()
    } else {
        (StatusCode::OK, "No mock recompile in progress").into_response()
    }
}

pub(super) async fn stop_mock_plugin_build() -> impl IntoResponse {
    if stop_mock_plugin_build_internal() {
        (StatusCode::ACCEPTED, "Stopping mock build").into_response()
    } else {
        (StatusCode::OK, "No mock build in progress").into_response()
    }
}

pub(super) async fn mock_plugin_build(State(state): State<AppState>) -> impl IntoResponse {
    let events = state.daemon.events.clone();
    let config_dir = crate::paths::shared_config_dir().ok();

    match start_mock_plugin_build(events, config_dir) {
        Ok(()) => (StatusCode::ACCEPTED, "Mock build queued").into_response(),
        Err(msg) => (StatusCode::CONFLICT, msg).into_response(),
    }
}
