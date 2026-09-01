use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use qol_plugin_daemon::notification::gate::NativeHandler;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::super::types::{AppState, MAX_CONFIG_SIZE};
use super::http_json::{self, blocking};

type HttpResult<T> = Result<T, Box<Response>>;

#[derive(Serialize)]
struct CoreConfigResponse {
    theme: String,
    accent: String,
    native_theme: String,
    profile: String,
    residency: bool,
    handler: NativeHandler,
}

#[derive(Deserialize)]
struct CoreConfigRequest {
    #[serde(flatten)]
    values: BTreeMap<String, serde_json::Value>,
}

fn current_config() -> CoreConfigResponse {
    CoreConfigResponse {
        theme: crate::features::theme::current_theme_key(),
        accent: crate::features::theme::current_accent_key(),
        native_theme: crate::features::theme::current_native_theme_key(),
        profile: crate::paths::active_profile_name(),
        residency: qol_host_fixes::residency::HostResidency::current().is_resident(),
        handler: crate::features::notifications::native_handler(),
    }
}

pub(super) fn do_not_disturb_status() -> String {
    match qol_plugin_daemon::notification::platform::os_do_not_disturb() {
        Some(true) => "Do not disturb: on".to_string(),
        Some(false) => "Do not disturb: off".to_string(),
        None => "Do not disturb: unknown".to_string(),
    }
}

pub(in super::super) async fn get_core_config() -> impl IntoResponse {
    blocking("core config", get_core_config_inner).await
}

pub(in super::super) async fn set_core_config(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking("core config", move || set_core_config_inner(&state, body)).await
}

fn get_core_config_inner() -> HttpResult<Response> {
    core_config_response(&current_config())
}

fn set_core_config_inner(state: &AppState, body: axum::body::Bytes) -> HttpResult<Response> {
    let request: CoreConfigRequest = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    for (field, value) in &request.values {
        dispatch_field(state, field, value)?;
    }
    core_config_response(&current_config())
}

fn dispatch_field(state: &AppState, field: &str, value: &serde_json::Value) -> HttpResult<()> {
    let result = match field {
        "theme" => crate::features::theme::save_selected_theme_key(&as_str(field, value)?),
        "accent" => crate::features::theme::save_selected_accent_key(&as_str(field, value)?),
        "native_theme" => {
            let previous = crate::features::theme::current_native_theme_key();
            let result =
                crate::features::theme::save_selected_native_theme_key(&as_str(field, value)?);
            if result.is_ok() && crate::features::theme::current_native_theme_key() != previous {
                super::theme_handlers::apply_theme_to_running_surfaces(state);
            }
            result
        }
        "profile" => crate::features::profile::registry::switch_active_profile(
            &state.daemon,
            &as_str(field, value)?,
        ),
        "residency" => set_residency(as_bool(field, value)?),
        "handler" => crate::features::notifications::set_native_handler(as_handler(field, value)?),
        _ => return Err(Box::new(bad_request(&format!("unknown field: {field}")))),
    };
    result.map_err(|error| Box::new(bad_request(&format!("{error:#}"))))
}

fn set_residency(resident: bool) -> anyhow::Result<()> {
    let value = if resident {
        qol_host_fixes::residency::HostResidency::Resident
    } else {
        qol_host_fixes::residency::HostResidency::Portable
    };
    crate::features::resident_policy::apply_residency(value)?;
    log::info!("core settings: residency set to {}", value.as_str());
    Ok(())
}

fn as_str(field: &str, value: &serde_json::Value) -> HttpResult<String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Box::new(bad_request(&format!("{field} expects a string"))))
}

fn as_bool(field: &str, value: &serde_json::Value) -> HttpResult<bool> {
    value
        .as_bool()
        .ok_or_else(|| Box::new(bad_request(&format!("{field} expects a boolean"))))
}

fn as_handler(field: &str, value: &serde_json::Value) -> HttpResult<NativeHandler> {
    match value.as_str() {
        Some("qol") => Ok(NativeHandler::Qol),
        Some("os") => Ok(NativeHandler::Os),
        Some("both") => Ok(NativeHandler::Both),
        _ => Err(Box::new(bad_request(&format!(
            "{field} expects qol, os, or both"
        )))),
    }
}

fn core_config_response(config: &CoreConfigResponse) -> HttpResult<Response> {
    let json = http_json::encode_json(config, "Failed to serialize core config")?;
    Ok(http_json::json_response(json))
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, message.to_string()).into_response()
}
