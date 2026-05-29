use axum::{
    extract::{FromRef, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use super::health::{auth_health, cumulative_scopes_for};
use super::types::AuthProvider;
use crate::features::github_auth::GitHubAuthService;

#[derive(Clone)]
pub(crate) struct AuthHttpState {
    pub(crate) github_auth_service: Arc<GitHubAuthService>,
}

#[derive(Debug, Deserialize)]
struct ReauthRequest {
    provider: AuthProvider,
}

pub(crate) fn routes<S>() -> Router<S>
where
    AuthHttpState: FromRef<S>,
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/auth/health", get(get_health))
        .route("/auth/reauth", post(start_reauth))
}

async fn get_health() -> Response {
    match tokio::task::spawn_blocking(auth_health).await {
        Ok(health) => Json(health).into_response(),
        Err(error) => {
            log::error!("auth health join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "auth health join error").into_response()
        }
    }
}

async fn start_reauth(
    State(state): State<AuthHttpState>,
    Json(body): Json<ReauthRequest>,
) -> Response {
    let scopes = cumulative_scopes_for(body.provider);
    match state.github_auth_service.start(&scopes).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}
