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
    tokio::task::spawn_blocking(move || {
        let date_str = log_date_str(query.date)?;
        let filename = format!("qol-tray.{}.log", date_str);
        let path = crate::logging::platform::log_dir().join(filename);

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        };

        Ok(content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    })
    .await
    .map_err(join_error_status)?
    .map(Json)
}

async fn suppressed() -> Result<Json<serde_json::Value>, StatusCode> {
    tokio::task::spawn_blocking(|| {
        let path = crate::paths::suppressed_errors_path()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(serde_json::json!({}));
            }
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        };

        serde_json::from_str(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    })
    .await
    .map_err(join_error_status)?
    .map(Json)
}

async fn unsuppress(Path(key): Path<String>) -> StatusCode {
    crate::logging::file_logger::unsuppress_key(&key);
    StatusCode::OK
}

async fn open_dir() -> StatusCode {
    tokio::task::spawn_blocking(|| {
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
    })
    .await
    .unwrap_or_else(|error| {
        log::error!("logs open_dir join error: {}", error);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn join_error_status(error: tokio::task::JoinError) -> StatusCode {
    log::error!("logs handler join error: {}", error);
    StatusCode::INTERNAL_SERVER_ERROR
}

fn today_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn log_date_str(date: Option<String>) -> Result<String, StatusCode> {
    let Some(date) = date else {
        return Ok(today_str());
    };
    if !is_strict_log_date(&date) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let parsed = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(parsed.format("%Y-%m-%d").to_string())
}

fn is_strict_log_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_date_str_accepts_valid_calendar_date() {
        assert_eq!(
            log_date_str(Some("2026-06-08".to_string())).unwrap(),
            "2026-06-08"
        );
    }

    #[test]
    fn log_date_str_rejects_path_traversal() {
        assert_eq!(
            log_date_str(Some("../../../tmp".to_string())).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn log_date_str_rejects_invalid_or_loose_dates() {
        for raw in ["2026-02-30", "2026-6-8", "20260608", "2026-06-08/extra"] {
            assert_eq!(
                log_date_str(Some(raw.to_string())).unwrap_err(),
                StatusCode::BAD_REQUEST,
                "raw={raw}"
            );
        }
    }
}
