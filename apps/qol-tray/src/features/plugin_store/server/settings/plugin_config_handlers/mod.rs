use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use super::super::helpers::validate_plugin_id_bad_request;
use super::super::types::AppState;

mod io;
mod notify;

type HttpResult<T> = Result<T, Response>;
const APPLICATION_JSON: &str = "application/json";

pub(in super::super) async fn get_plugin_config(
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    get_plugin_config_inner(plugin_id).unwrap_or_else(|response| response)
}

pub(in super::super) async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    set_plugin_config_inner(plugin_id, &state, body).unwrap_or_else(|response| response)
}

fn get_plugin_config_inner(plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = io::load_plugin_config(&plugin_id)?;
    Ok(config_json_response(&config))
}

fn set_plugin_config_inner(
    plugin_id: String,
    state: &AppState,
    body: axum::body::Bytes,
) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = io::parse_config_body(body)?;
    io::save_plugin_config(&plugin_id, config)?;
    notify::notify_plugin_reload(state, &plugin_id);
    Ok(config_saved_response())
}

fn validated_plugin_id(plugin_id: String) -> HttpResult<String> {
    validate_plugin_id_bad_request(&plugin_id).map_err(IntoResponse::into_response)?;
    Ok(plugin_id)
}

fn config_json_response(config: &serde_json::Value) -> Response {
    let Ok(json) = io::encode_config_json(config) else {
        return serialize_config_failed_response();
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, APPLICATION_JSON)],
        json,
    )
        .into_response()
}

fn config_saved_response() -> Response {
    (StatusCode::OK, "Config saved").into_response()
}

fn serialize_config_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize config",
    )
        .into_response()
}
