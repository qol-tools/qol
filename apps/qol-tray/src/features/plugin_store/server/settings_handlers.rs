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

pub(super) async fn serve_icon(Path(bundle_id): Path<String>) -> impl IntoResponse {
    let icon = tokio::task::spawn_blocking(move || {
        qol_plugin_api::app_icon::icon_for_bundle_id(&bundle_id, 32)
    })
    .await
    .ok()
    .flatten();

    let Some(icon) = icon else {
        return (StatusCode::NOT_FOUND, "Icon not found").into_response();
    };

    let mut rgba = icon.data;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    let mut png_buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_buf, icon.width as u32, icon.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let Ok(mut writer) = encoder.write_header() else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "PNG encode failed").into_response();
        };
        if writer.write_image_data(&rgba).is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "PNG encode failed").into_response();
        }
    }

    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], png_buf).into_response()
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
    State(state): State<AppState>,
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
            notify_plugin_reload(&state, &plugin_id);
            (StatusCode::OK, "Config saved").into_response()
        }
        Err(e) => {
            log::error!("Failed to save config: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config").into_response()
        }
    }
}

fn notify_plugin_reload(state: &AppState, plugin_id: &str) {
    let socket_path = {
        let manager = state.plugin_manager.lock().unwrap();
        manager.get(plugin_id).and_then(|p| {
            p.manifest.daemon.as_ref()?.socket.clone()
        })
    };

    #[cfg(unix)]
    if let Some(path) = socket_path {
        use std::io::Write;
        use std::os::unix::net::UnixStream;

        match UnixStream::connect(&path) {
            Ok(mut stream) => {
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(500)));
                if stream.write_all(b"reload").is_ok() {
                    log::info!("Sent reload to plugin {} via {}", plugin_id, path);
                }
            }
            Err(e) => {
                log::debug!("Could not connect to plugin socket {}: {}", path, e);
            }
        }
    }
}

pub(super) async fn list_apps() -> Json<Vec<serde_json::Value>> {
    let apps = tokio::task::spawn_blocking(discover_installed_apps)
        .await
        .unwrap_or_default();
    Json(apps)
}

#[cfg(target_os = "macos")]
fn discover_installed_apps() -> Vec<serde_json::Value> {
    use std::process::Command;

    let mdfind = Command::new("mdfind")
        .args([
            "-onlyin", "/Applications",
            "-onlyin", "/System/Applications",
        ])
        .arg("kMDItemContentType == 'com.apple.application-bundle'")
        .output();

    let Ok(output) = mdfind else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut apps: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|path| {
            let app_path = std::path::Path::new(path);
            if !app_path.is_dir() {
                return None;
            }
            // Skip paths inside bundle Contents (spotlight sometimes returns nested)
            if app_path.components().any(|c| c.as_os_str() == "Contents") {
                return None;
            }
            let name = app_path.file_stem()?.to_str()?.to_string();
            let bid = Command::new("defaults")
                .args(["read", &format!("{}/Contents/Info", path), "CFBundleIdentifier"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;
            if bid.is_empty() {
                return None;
            }
            Some(serde_json::json!({ "bundle_id": bid, "name": name }))
        })
        .collect();

    apps.sort_by(|a, b| {
        let na = a["name"].as_str().unwrap_or("");
        let nb = b["name"].as_str().unwrap_or("");
        na.to_lowercase().cmp(&nb.to_lowercase())
    });
    apps.dedup_by(|a, b| a["bundle_id"] == b["bundle_id"]);
    apps
}

#[cfg(not(target_os = "macos"))]
fn discover_installed_apps() -> Vec<serde_json::Value> {
    Vec::new()
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
