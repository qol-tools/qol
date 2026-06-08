use axum::{http::StatusCode, routing::post, Json, Router};
use serde::Deserialize;
use std::collections::BTreeMap;

use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/trace/ui", post(trace_ui))
}

#[derive(Deserialize)]
struct UiTraceBody {
    event: String,
    #[serde(default)]
    fields: BTreeMap<String, String>,
}

async fn trace_ui(Json(body): Json<UiTraceBody>) -> StatusCode {
    trace_ui_event(&body.event, &body.fields);
    StatusCode::NO_CONTENT
}

#[cfg(debug_assertions)]
fn trace_ui_event(event: &str, fields: &BTreeMap<String, String>) {
    let msg = format_fields(fields);
    match event {
        "route" => qol_runtime::probe!("WORLD_ROUTE", "{msg}"),
        "dive" => qol_runtime::probe!("WORLD_DIVE", "{msg}"),
        _ => {
            let event = sanitize_value(event);
            qol_runtime::probe!("WORLD_UI", "event={event} {msg}");
        }
    }
}

#[cfg(not(debug_assertions))]
fn trace_ui_event(event: &str, fields: &BTreeMap<String, String>) {
    let _ = (event, fields);
}

#[cfg(debug_assertions)]
fn format_fields(fields: &BTreeMap<String, String>) -> String {
    fields
        .iter()
        .take(24)
        .filter_map(|(key, value)| {
            sanitize_key(key).map(|key| format!("{key}={}", sanitize_value(value)))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(debug_assertions)]
fn sanitize_key(key: &str) -> Option<String> {
    let key = key.trim();
    if key.is_empty() || key.len() > 48 {
        return None;
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(key.to_string())
}

#[cfg(debug_assertions)]
fn sanitize_value(value: &str) -> String {
    value
        .chars()
        .take(160)
        .map(|c| {
            if c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | '#' | ':' | '=' | '@' | ',')
            {
                c
            } else {
                '_'
            }
        })
        .collect()
}
