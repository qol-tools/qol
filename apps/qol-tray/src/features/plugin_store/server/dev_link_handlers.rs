#![cfg(feature = "dev")]

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::dev;
use std::path::PathBuf;

use super::dev_services;
use super::helpers::{
    shared_config_dir, shared_config_dir_or_response, shared_config_dir_or_status,
    validate_plugin_id_bad_request,
};
use super::types::{AppState, UpsertPluginLogControlRequest};

fn config_dir() -> Result<PathBuf, (StatusCode, String)> {
    shared_config_dir_or_response("Config dir unavailable")
}

pub(super) async fn list_linked_plugins(
    State(_state): State<AppState>,
) -> Result<Json<Vec<dev::LinkedPlugin>>, StatusCode> {
    let config_dir = shared_config_dir_or_status()?;
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
            dev_services::refresh_discovery(&state);
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
    if let Err(error) = validate_plugin_id_bad_request(&id) {
        return error.into_response();
    }

    let config_dir = match config_dir() {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    match dev::remove_link(&id, &config_dir) {
        Ok(()) => {
            dev_services::refresh_discovery(&state);
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
    if let Err(error) = validate_plugin_id_bad_request(&id) {
        return error.into_response();
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

pub(super) async fn get_log_controls(
) -> Json<std::collections::HashMap<String, crate::plugins::log_control::PluginLogControl>> {
    let controls = shared_config_dir()
        .ok()
        .map(|dir| crate::plugins::log_control::load_all_controls(&dir))
        .unwrap_or_default();
    Json(controls)
}
