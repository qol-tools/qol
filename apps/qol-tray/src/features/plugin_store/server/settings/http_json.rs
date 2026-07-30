use axum::{
    body::Bytes,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub(super) async fn blocking<F>(label: &'static str, work: F) -> Response
where
    F: FnOnce() -> Result<Response, Box<Response>> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(response)) => response,
        Ok(Err(boxed)) => *boxed,
        Err(error) => {
            log::error!("{label} handler join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "Handler crashed").into_response()
        }
    }
}

pub(super) fn parse_json_body<T: DeserializeOwned>(
    body: Bytes,
    max_size: usize,
) -> Result<T, Box<Response>> {
    if body.len() > max_size {
        return Err(Box::new(config_too_large_response()));
    }
    serde_json::from_slice(&body).map_err(|_| Box::new(invalid_json_response()))
}

pub(super) fn encode_json<T: Serialize>(
    value: &T,
    error_message: &'static str,
) -> Result<Vec<u8>, Box<Response>> {
    serde_json::to_vec(value).map_err(|_| Box::new(serialize_failed_response(error_message)))
}

pub(super) fn json_response(json: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json,
    )
        .into_response()
}

fn invalid_json_response() -> Response {
    (StatusCode::BAD_REQUEST, "Invalid JSON").into_response()
}

fn config_too_large_response() -> Response {
    (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response()
}

fn serialize_failed_response(message: &'static str) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
}
