use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use super::super::helpers::validate_plugin_id_bad_request;
use super::super::types::AppState;

mod cover_file;

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn serve_cover(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    serve_cover_inner(&state, plugin_id)
        .await
        .unwrap_or_else(|response| *response)
}

async fn serve_cover_inner(state: &AppState, plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let data = cover_file::load_cover_bytes(&state.plugins_dir, &plugin_id).await?;
    Ok((StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], data).into_response())
}

fn validated_plugin_id(plugin_id: String) -> HttpResult<String> {
    validate_plugin_id_bad_request(&plugin_id).map_err(|e| Box::new(e.into_response()))?;
    Ok(plugin_id)
}
