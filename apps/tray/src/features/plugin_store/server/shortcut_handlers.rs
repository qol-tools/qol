use axum::{extract::Path, http::StatusCode, routing::post, Router};

use super::types::AppState;
use crate::shortcuts::executor::ExecuteByIdError;

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/shortcuts/{id}/execute", post(execute_shortcut))
}

async fn execute_shortcut(Path(id): Path<String>) -> (StatusCode, String) {
    let result =
        tokio::task::spawn_blocking(move || crate::shortcuts::executor::execute_by_id(&id)).await;
    match result {
        Ok(Ok(())) => (StatusCode::OK, "Shortcut executed".to_string()),
        Ok(Err(ExecuteByIdError::InvalidId(message))) => (StatusCode::BAD_REQUEST, message),
        Ok(Err(ExecuteByIdError::NotFound(id))) => {
            (StatusCode::NOT_FOUND, format!("shortcut '{id}' not found"))
        }
        Ok(Err(error)) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        Err(error) => {
            log::error!("Shortcut execution handler crashed: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Handler crashed".to_string(),
            )
        }
    }
}
