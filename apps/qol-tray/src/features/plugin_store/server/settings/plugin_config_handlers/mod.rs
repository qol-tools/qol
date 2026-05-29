use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::super::helpers::validate_plugin_id_bad_request;
use super::super::types::AppState;
use super::http_json;

mod form;
mod io;
mod notify;

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn get_plugin_config(
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    blocking(move || get_plugin_config_inner(plugin_id)).await
}

pub(in super::super) async fn get_plugin_config_form(
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    blocking(move || get_plugin_config_form_inner(plugin_id)).await
}

pub(in super::super) async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking(move || set_plugin_config_inner(plugin_id, &state, body)).await
}

async fn blocking<F>(work: F) -> Response
where
    F: FnOnce() -> HttpResult<Response> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(response)) => response,
        Ok(Err(boxed)) => *boxed,
        Err(error) => {
            log::error!("plugin config handler join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "Handler crashed").into_response()
        }
    }
}

fn get_plugin_config_inner(plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = io::load_plugin_config(&plugin_id)?;
    Ok(config_json_response(&config))
}

fn get_plugin_config_form_inner(plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let form = form::load_plugin_config_form(&plugin_id)?;
    Ok(config_form_json_response(&form))
}

fn set_plugin_config_inner(
    plugin_id: String,
    state: &AppState,
    body: axum::body::Bytes,
) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = io::parse_config_body(body)?;
    form::validate_plugin_config(&plugin_id, &config)?;
    io::save_plugin_config(&plugin_id, config)?;
    if let Err(error) = notify::notify_plugin_reload(state, &plugin_id) {
        log::error!(
            "Config saved for {}, but daemon refresh failed: {}",
            plugin_id,
            error
        );
        return Err(Box::new(config_refresh_failed_response()));
    }
    Ok(config_saved_response())
}

fn validated_plugin_id(plugin_id: String) -> HttpResult<String> {
    validate_plugin_id_bad_request(&plugin_id).map_err(|e| Box::new(e.into_response()))?;
    Ok(plugin_id)
}

fn config_json_response(config: &serde_json::Value) -> Response {
    let json = match io::encode_config_json(config) {
        Ok(json) => json,
        Err(_) => return serialize_config_failed_response(),
    };
    http_json::json_response(json)
}

fn config_form_json_response(combined: &form::CombinedPluginForm) -> Response {
    let json = match http_json::encode_json(combined, "Failed to serialize config form") {
        Ok(json) => json,
        Err(_) => return serialize_config_form_failed_response(),
    };
    http_json::json_response(json)
}

fn config_saved_response() -> Response {
    (StatusCode::OK, "Config saved").into_response()
}

fn config_refresh_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Config saved but live daemon refresh failed",
    )
        .into_response()
}

fn serialize_config_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize config",
    )
        .into_response()
}

fn serialize_config_form_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize config form",
    )
        .into_response()
}
