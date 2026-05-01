use axum::{
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use std::sync::Arc;

use super::{GitHubAuthService, GitHubAuthStatus};

#[derive(Clone)]
pub(crate) struct GitHubAuthHttpState {
    pub(crate) github_auth_service: Arc<GitHubAuthService>,
}

pub(crate) fn routes<S>() -> Router<S>
where
    GitHubAuthHttpState: FromRef<S>,
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/github-auth/status", get(get_status))
        .route("/github-auth/start", post(start_auth))
        .route("/github-auth/poll/{id}", post(poll_session))
        .route("/github-auth", delete(disconnect))
}

async fn get_status(State(state): State<GitHubAuthHttpState>) -> Json<GitHubAuthStatus> {
    Json(state.github_auth_service.status())
}

async fn start_auth(State(state): State<GitHubAuthHttpState>) -> impl IntoResponse {
    match state.github_auth_service.start().await {
        Ok(response) => Json(response).into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn poll_session(
    State(state): State<GitHubAuthHttpState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    Json(state.github_auth_service.poll_session(&id).await)
}

async fn disconnect(State(state): State<GitHubAuthHttpState>) -> impl IntoResponse {
    match state.github_auth_service.disconnect() {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => auth_error_response(error),
    }
}

fn auth_error_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let normalized = message.to_lowercase();
    let status = if normalized.contains("not configured")
        || normalized.contains("missing")
        || normalized.contains("invalid")
    {
        StatusCode::BAD_REQUEST
    } else if normalized.contains("github") || normalized.contains("oauth") {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, message).into_response()
}
