use axum::{extract::State, http::StatusCode, response::IntoResponse, response::Response, Json};

use super::super::helpers::reload_manager_and_notify_without_profile_sync;
use super::super::types::AppState;

pub(in super::super) async fn export_config(State(state): State<AppState>) -> impl IntoResponse {
    let plugins = export_plugins(&state);
    let bundle = crate::profile::build_export_bundle(chrono::Local::now().to_rfc3339(), plugins);
    match bundle {
        Ok(bundle) => Json(bundle).into_response(),
        Err(error) => server_error(error),
    }
}

pub(in super::super) async fn import_config(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let bundle: crate::profile::ProfileImportBundle = match serde_json::from_slice(&body) {
        Ok(bundle) => bundle,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match import_bundle(&state, bundle).await {
        Ok(result) => Json(result).into_response(),
        Err(response) => response,
    }
}

async fn import_bundle(
    state: &AppState,
    bundle: crate::profile::ProfileImportBundle,
) -> Result<crate::profile::ApplyProfileResult, Response> {
    let result = crate::profile::apply_import_bundle(&state.plugins_dir, &bundle)
        .await
        .map_err(server_error)?;
    reload_manager_and_notify_without_profile_sync(state);
    Ok(result)
}

fn export_plugins(state: &AppState) -> Vec<crate::profile::PluginLockEntry> {
    let Ok(manager) = state.plugin_manager.lock() else {
        return crate::profile::load_plugins_lock()
            .map(|lock| lock.plugins)
            .unwrap_or_default();
    };
    crate::profile::sync_plugins_lock_from_plugins(manager.plugins())
        .map(|lock| lock.plugins)
        .unwrap_or_else(|_| {
            crate::profile::load_plugins_lock()
                .map(|lock| lock.plugins)
                .unwrap_or_default()
        })
}

fn server_error(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to import profile: {error:#}"),
    )
        .into_response()
}
