use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::super::types::MAX_CONFIG_SIZE;
use super::http_json::{self, blocking};

type HttpResult<T> = Result<T, Box<Response>>;

#[derive(Deserialize)]
struct NotificationsRequest {
    #[serde(rename = "useSystemNotifications")]
    use_system_notifications: bool,
}

#[derive(Serialize)]
struct NotificationsResponse {
    #[serde(rename = "useSystemNotifications")]
    use_system_notifications: bool,
}

pub(in super::super) async fn get_notifications() -> impl IntoResponse {
    blocking("notifications", notifications_response).await
}

pub(in super::super) async fn set_notifications(body: axum::body::Bytes) -> impl IntoResponse {
    blocking("notifications", move || set_notifications_inner(body)).await
}

fn set_notifications_inner(body: axum::body::Bytes) -> HttpResult<Response> {
    let request: NotificationsRequest = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    crate::features::notifications::set_use_system_notifications(request.use_system_notifications)
        .map_err(|error| Box::new(bad_request(&error.to_string())))?;
    notifications_response()
}

fn notifications_response() -> HttpResult<Response> {
    let body = NotificationsResponse {
        use_system_notifications: crate::features::notifications::use_system_notifications(),
    };
    let json = http_json::encode_json(&body, "Failed to serialize notification settings")?;
    Ok(http_json::json_response(json))
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, message.to_string()).into_response()
}
