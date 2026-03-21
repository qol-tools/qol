use axum::{
    extract::{Path, Query},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::collections::HashMap;

use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/logs/entries", get(entries))
        .route("/logs/suppressed", get(suppressed))
        .route("/logs/unsuppress/{key}", post(unsuppress))
}

#[derive(Deserialize, Default)]
struct EntriesQuery {
    date: Option<String>,
}

async fn entries(
    Query(query): Query<EntriesQuery>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let date_str = query.date.unwrap_or_else(today_str);
    let filename = format!("qol-tray.{}.log", date_str);
    let path = crate::logging::platform::log_dir().join(filename);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(Vec::new())),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let entries: Vec<serde_json::Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    Ok(Json(entries))
}

async fn suppressed() -> Result<Json<serde_json::Value>, StatusCode> {
    let path =
        crate::paths::suppressed_errors_path().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(serde_json::json!({})));
        }
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(value))
}

async fn unsuppress(Path(key): Path<String>) -> Result<StatusCode, StatusCode> {
    let path =
        crate::paths::suppressed_errors_path().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(StatusCode::OK),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let mut map: HashMap<String, serde_json::Value> =
        serde_json::from_str(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    map.remove(&key);

    if map.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        let updated =
            serde_json::to_string_pretty(&map).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        std::fs::write(&path, updated).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(StatusCode::OK)
}

fn today_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
