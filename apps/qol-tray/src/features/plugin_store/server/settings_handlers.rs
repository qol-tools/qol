use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};

use crate::hotkeys::trigger_reload;
use crate::paths::is_safe_path_component;
use crate::plugins::PluginConfigManager;

use super::types::{AppState, TokenRequest, TokenStatus, MAX_CONFIG_SIZE, MAX_COVER_SIZE};

pub(super) async fn serve_cover(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    let plugin_root = state.plugins_dir.join(&plugin_id);
    let plugin_meta = match tokio::fs::symlink_metadata(&plugin_root).await {
        Ok(meta) => meta,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    if plugin_meta.file_type().is_symlink() {
        return (StatusCode::FORBIDDEN, "Invalid cover path").into_response();
    }

    let cover_path = plugin_root.join("cover.png");
    let cover_meta = match tokio::fs::symlink_metadata(&cover_path).await {
        Ok(meta) => meta,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    if cover_meta.file_type().is_symlink() {
        return (StatusCode::FORBIDDEN, "Invalid cover path").into_response();
    }
    if !cover_meta.is_file() {
        return (StatusCode::NOT_FOUND, "Cover not found").into_response();
    }

    let canonical_root = match tokio::fs::canonicalize(&plugin_root).await {
        Ok(path) => path,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    let canonical_cover = match tokio::fs::canonicalize(&cover_path).await {
        Ok(path) => path,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    if !canonical_cover.starts_with(&canonical_root) {
        return (StatusCode::FORBIDDEN, "Invalid cover path").into_response();
    }

    let cover_size = match tokio::fs::metadata(&canonical_cover).await {
        Ok(meta) => meta.len() as usize,
        Err(e) => {
            log::error!("Failed to read cover metadata: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read cover").into_response();
        }
    };

    if cover_size > MAX_COVER_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Cover image too large").into_response();
    }

    let data = match tokio::fs::read(&canonical_cover).await {
        Ok(data) => data,
        Err(e) => {
            log::error!("Failed to read cover image: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read cover").into_response();
        }
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], data).into_response()
}

pub(super) async fn get_plugin_config(Path(plugin_id): Path<String>) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    let config = match PluginConfigManager::new().and_then(|m| m.get_config(&plugin_id)) {
        Ok(Some(config)) => config,
        Ok(None) => return (StatusCode::NOT_FOUND, "Config not found").into_response(),
        Err(e) => {
            log::error!("Failed to read config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read config").into_response();
        }
    };

    match serde_json::to_vec(&config) {
        Ok(data) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            data,
        )
            .into_response(),
        Err(e) => {
            log::error!("Failed to serialize config: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to serialize config",
            )
                .into_response()
        }
    }
}

pub(super) async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    if body.len() > MAX_CONFIG_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response();
    }

    let config: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Invalid JSON in config: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response();
        }
    };

    match PluginConfigManager::new().and_then(|m| m.set_config(&plugin_id, config)) {
        Ok(()) => {
            log::info!("Config saved for plugin: {}", plugin_id);
            (StatusCode::OK, "Config saved").into_response()
        }
        Err(e) => {
            log::error!("Failed to save config: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config").into_response()
        }
    }
}

pub(super) async fn get_token_status() -> Json<TokenStatus> {
    Json(TokenStatus {
        has_token: super::super::github::get_stored_token().is_some(),
    })
}

pub(super) async fn set_github_token(Json(payload): Json<TokenRequest>) -> impl IntoResponse {
    use super::super::github::TokenValidationError;

    if let Err(e) = super::super::github::validate_token(&payload.token).await {
        let (status, label) = match &e {
            TokenValidationError::Empty | TokenValidationError::Invalid(_) => {
                (StatusCode::BAD_REQUEST, "Rejected")
            }
            TokenValidationError::Upstream(_) => (StatusCode::BAD_GATEWAY, "Upstream failure"),
        };
        log::warn!("{} GitHub token: {}", label, e);
        return (status, e.to_string()).into_response();
    }

    if let Err(e) = super::super::github::store_token(&payload.token) {
        log::error!("Failed to store GitHub token: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store token".to_string(),
        )
            .into_response();
    }

    log::info!("GitHub token stored successfully");
    (StatusCode::OK, "Token stored".to_string()).into_response()
}

pub(super) async fn delete_github_token() -> impl IntoResponse {
    if let Err(e) = super::super::github::delete_token() {
        log::error!("Failed to delete GitHub token: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to delete token".to_string(),
        )
            .into_response();
    }

    log::info!("GitHub token deleted");
    (StatusCode::OK, "Token deleted".to_string()).into_response()
}

pub(super) async fn get_hotkeys() -> impl IntoResponse {
    use crate::hotkeys::HotkeyManager;

    let manager = match HotkeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to create HotkeyManager: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response();
        }
    };

    let config = match manager.load_config() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to load hotkey config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response();
        }
    };

    let json = match serde_json::to_vec(&config) {
        Ok(j) => j,
        Err(e) => {
            log::error!("Failed to serialize hotkey config: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to serialize hotkeys",
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json,
    )
        .into_response()
}

pub(super) async fn set_hotkeys(body: axum::body::Bytes) -> impl IntoResponse {
    use crate::hotkeys::{HotkeyConfig, HotkeyManager};

    if body.len() > MAX_CONFIG_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response();
    }

    let config: HotkeyConfig = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Invalid hotkey config JSON: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response();
        }
    };

    let manager = match HotkeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to create HotkeyManager: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save hotkeys").into_response();
        }
    };

    if let Err(e) = manager.save_config(&config) {
        log::error!("Failed to save hotkey config: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save hotkeys").into_response();
    }

    trigger_reload();
    log::info!("Hotkey config saved");
    (StatusCode::OK, "Hotkeys saved").into_response()
}
