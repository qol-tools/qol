use axum::{
    body::Bytes,
    extract::Path,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

pub(crate) async fn get_sync_status(State(state): State<super::ProfileHttpState>) -> Response {
    match tokio::task::spawn_blocking(move || state.sync_service.status()).await {
        Ok(status) => Json(status).into_response(),
        Err(error) => {
            log::error!("get_sync_status join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "sync status join error").into_response()
        }
    }
}

pub(crate) async fn get_sync_providers(
    State(_state): State<super::ProfileHttpState>,
) -> impl IntoResponse {
    Json(serde_json::json!([]))
}

pub(crate) async fn connect_sync(
    State(state): State<super::ProfileHttpState>,
    body: Bytes,
) -> impl IntoResponse {
    let request =
        match super::parse_json_body::<crate::features::profile::sync::SyncConnectRequest>(body) {
            Ok(request) => request,
            Err(response) => return *response,
        };
    match state.sync_service.connect(request).await {
        Ok(result) => sync_result_response(&state, result),
        Err(error) => sync_error_response(error),
    }
}

pub(crate) async fn bootstrap_sync_github(
    State(state): State<super::ProfileHttpState>,
) -> impl IntoResponse {
    match state.sync_service.bootstrap_github_connect().await {
        Ok(result) => sync_result_response(&state, result),
        Err(error) => sync_error_response(error),
    }
}

pub(crate) async fn pull_sync(State(state): State<super::ProfileHttpState>) -> impl IntoResponse {
    match state.sync_service.manual_pull().await {
        Ok(result) => sync_result_response(&state, result),
        Err(error) => sync_error_response(error),
    }
}

pub(crate) async fn push_sync(State(state): State<super::ProfileHttpState>) -> impl IntoResponse {
    match state.sync_service.manual_push().await {
        Ok(result) => sync_result_response(&state, result),
        Err(error) => sync_error_response(error),
    }
}

pub(crate) async fn disconnect_sync(
    State(state): State<super::ProfileHttpState>,
) -> impl IntoResponse {
    match state.sync_service.disconnect().await {
        Ok(result) => Json(result).into_response(),
        Err(error) => sync_error_response(error),
    }
}

pub(crate) async fn acknowledge_sync(
    State(state): State<super::ProfileHttpState>,
) -> impl IntoResponse {
    match state.sync_service.acknowledge_incident().await {
        Ok(result) => Json(result).into_response(),
        Err(error) => sync_error_response(error),
    }
}

pub(crate) async fn open_sync_backups_dir(
    State(state): State<super::ProfileHttpState>,
) -> Response {
    blocking_sync_response(
        move || state.sync_service.open_backups_dir(),
        |()| StatusCode::OK.into_response(),
    )
    .await
}

pub(crate) async fn open_sync_backup_file(
    State(state): State<super::ProfileHttpState>,
    Path(file_name): Path<String>,
) -> Response {
    blocking_sync_response(
        move || state.sync_service.open_backup_file(&file_name),
        |()| StatusCode::OK.into_response(),
    )
    .await
}

pub(crate) async fn list_sync_backups(State(state): State<super::ProfileHttpState>) -> Response {
    blocking_sync_response(
        move || state.sync_service.list_backups(),
        |backups| Json(backups).into_response(),
    )
    .await
}

pub(crate) async fn preview_sync_backup(
    State(state): State<super::ProfileHttpState>,
    Path(file_name): Path<String>,
) -> Response {
    blocking_sync_response(
        move || state.sync_service.preview_backup(&file_name),
        |preview| Json(preview).into_response(),
    )
    .await
}

async fn blocking_sync_response<T, F, M>(work: F, into_ok: M) -> Response
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    M: FnOnce(T) -> Response,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(value)) => into_ok(value),
        Ok(Err(error)) => sync_error_response(error),
        Err(error) => {
            log::error!("sync backup join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "sync backup join error").into_response()
        }
    }
}

fn sync_result_response(
    state: &super::ProfileHttpState,
    result: crate::features::profile::sync::SyncActionResult,
) -> Response {
    if result.applied_remote {
        super::reload_after_profile_apply(state);
    }
    Json(result).into_response()
}

fn sync_error_response(error: anyhow::Error) -> Response {
    let message = format!("{:#}", error);
    log::error!("sync error: {:#}", error);
    let status = if looks_like_bad_request(&message) {
        StatusCode::BAD_REQUEST
    } else if looks_like_upstream_error(&message) {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, message).into_response()
}

fn looks_like_bad_request(message: &str) -> bool {
    let normalized = message.to_lowercase();
    normalized.contains("required")
        || normalized.contains("invalid")
        || normalized.contains("not configured")
        || normalized.contains("not connected")
        || normalized.contains("cannot be empty")
        || normalized.contains("unsupported")
}

fn looks_like_upstream_error(message: &str) -> bool {
    let normalized = message.to_lowercase();
    normalized.contains("github")
        || normalized.contains("upstream")
        || normalized.contains("authentication failed")
}
