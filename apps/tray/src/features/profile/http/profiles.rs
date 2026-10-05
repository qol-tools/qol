use axum::{
    body::Bytes,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::features::profile::registry;

#[derive(serde::Deserialize)]
struct CreateProfileRequest {
    name: String,
}

pub(crate) async fn list_profiles() -> Response {
    match tokio::task::spawn_blocking(registry::profile_summaries).await {
        Ok(Ok(profiles)) => Json(profiles).into_response(),
        Ok(Err(error)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")).into_response(),
        Err(error) => {
            log::error!("list_profiles join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "profile list join error").into_response()
        }
    }
}

pub(crate) async fn create_profile(body: Bytes) -> Response {
    let request = match super::parse_json_body::<CreateProfileRequest>(body) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    let created = tokio::task::spawn_blocking(move || {
        registry::create_profile_from_active(&request.name)
            .and_then(|()| registry::profile_summaries())
    })
    .await;
    match created {
        Ok(Ok(profiles)) => Json(profiles).into_response(),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, format!("{error:#}")).into_response(),
        Err(error) => {
            log::error!("create_profile join error: {}", error);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "create profile join error",
            )
                .into_response()
        }
    }
}
