use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use super::super::super::github;
use super::super::types::{TokenRequest, TokenStatus};

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn get_token_status() -> Json<TokenStatus> {
    Json(TokenStatus {
        has_token: github::get_stored_token().is_some(),
    })
}

pub(in super::super) async fn set_github_token(
    Json(payload): Json<TokenRequest>,
) -> impl IntoResponse {
    set_github_token_inner(payload.token)
        .await
        .unwrap_or_else(|response| *response)
}

pub(in super::super) async fn delete_github_token() -> impl IntoResponse {
    delete_github_token_inner().unwrap_or_else(|response| *response)
}

async fn set_github_token_inner(token: String) -> HttpResult<Response> {
    github::validate_token(&token)
        .await
        .map_err(|e| Box::new(token_validation_response(e)))?;
    github::store_token(&token).map_err(|_| Box::new(store_failed_response()))?;
    Ok(token_stored_response())
}

fn delete_github_token_inner() -> HttpResult<Response> {
    github::delete_token().map_err(|_| Box::new(delete_failed_response()))?;
    Ok(token_deleted_response())
}

fn token_validation_response(error: github::TokenValidationError) -> Response {
    use github::TokenValidationError;

    let status = match error {
        TokenValidationError::Empty | TokenValidationError::Invalid(_) => StatusCode::BAD_REQUEST,
        TokenValidationError::Upstream(_) => StatusCode::BAD_GATEWAY,
    };
    (status, error.to_string()).into_response()
}

fn token_stored_response() -> Response {
    (StatusCode::OK, "Token stored").into_response()
}

fn token_deleted_response() -> Response {
    (StatusCode::OK, "Token deleted").into_response()
}

fn store_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to store token").into_response()
}

fn delete_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete token").into_response()
}
