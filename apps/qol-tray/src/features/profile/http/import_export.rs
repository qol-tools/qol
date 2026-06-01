use axum::{extract::State, http::StatusCode, response::IntoResponse, response::Response, Json};

pub(crate) async fn export_config(
    State(state): State<super::ProfileHttpState>,
) -> impl IntoResponse {
    let plugins = export_plugins(&state);
    let bundle = crate::features::profile::core::build_export_bundle(
        chrono::Local::now().to_rfc3339(),
        plugins,
    );
    match bundle {
        Ok(bundle) => Json(bundle).into_response(),
        Err(error) => export_server_error(error),
    }
}

pub(crate) async fn import_config(
    State(state): State<super::ProfileHttpState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let bundle =
        match super::parse_json_body::<crate::features::profile::core::ProfileImportBundle>(body) {
            Ok(bundle) => bundle,
            Err(response) => return *response,
        };
    match import_bundle(&state, bundle).await {
        Ok(result) => Json(result).into_response(),
        Err(response) => response,
    }
}

async fn import_bundle(
    state: &super::ProfileHttpState,
    bundle: crate::features::profile::core::ProfileImportBundle,
) -> Result<crate::features::profile::core::ApplyProfileResult, Response> {
    let result = crate::features::profile::core::apply_import_bundle(&state.plugins_dir, &bundle)
        .await
        .map_err(import_server_error)?;
    super::reload_after_profile_apply(state);
    Ok(result)
}

fn export_plugins(
    state: &super::ProfileHttpState,
) -> Vec<crate::features::profile::core::PluginLockEntry> {
    let Ok(manager) = state.plugin_manager.lock() else {
        return crate::features::profile::core::load_plugins_lock()
            .map(|lock| lock.plugins)
            .unwrap_or_default();
    };
    crate::features::profile::core::sync_plugins_lock_from_plugins(manager.plugins())
        .map(|lock| lock.plugins)
        .unwrap_or_else(|_| {
            crate::features::profile::core::load_plugins_lock()
                .map(|lock| lock.plugins)
                .unwrap_or_default()
        })
}

fn export_server_error(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to export profile: {error:#}"),
    )
        .into_response()
}

fn import_server_error(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to import profile: {error:#}"),
    )
        .into_response()
}
