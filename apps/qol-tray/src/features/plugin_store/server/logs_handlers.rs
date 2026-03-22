use axum::{
    extract::{Path, Query},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/logs/entries", get(entries))
        .route("/logs/suppressed", get(suppressed))
        .route("/logs/unsuppress/{key}", post(unsuppress))
        .route("/logs/open-dir", post(open_dir))
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

async fn unsuppress(Path(key): Path<String>) -> StatusCode {
    crate::logging::file_logger::unsuppress_key(&key);
    StatusCode::OK
}

async fn open_dir() -> StatusCode {
    let dir = crate::logging::platform::log_dir();
    if !dir.exists() {
        return StatusCode::NOT_FOUND;
    }

    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(&dir).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&dir).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(&dir).spawn();

    match result {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn today_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
