use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use std::path::PathBuf;

use super::super::helpers::validate_plugin_id_bad_request;
use super::super::types::{AppState, MAX_COVER_SIZE};

type HttpResult<T> = Result<T, Response>;

struct CoverPaths {
    plugin_root: PathBuf,
    cover_path: PathBuf,
}

pub(in super::super) async fn serve_cover(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    serve_cover_inner(&state, plugin_id)
        .await
        .unwrap_or_else(|response| response)
}

async fn serve_cover_inner(state: &AppState, plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let paths = cover_paths(&state.plugins_dir, &plugin_id).await?;
    let canonical_cover = canonical_cover_path(&paths).await?;
    let data = read_cover_bytes(&canonical_cover).await?;
    Ok(cover_png_response(data))
}

fn validated_plugin_id(plugin_id: String) -> HttpResult<String> {
    validate_plugin_id_bad_request(&plugin_id).map_err(IntoResponse::into_response)?;
    Ok(plugin_id)
}

async fn cover_paths(base_dir: &PathBuf, plugin_id: &str) -> HttpResult<CoverPaths> {
    let plugin_root = base_dir.join(plugin_id);
    ensure_existing_non_symlink(&plugin_root).await?;
    let cover_path = plugin_root.join("cover.png");
    ensure_cover_file(&cover_path).await?;
    Ok(CoverPaths {
        plugin_root,
        cover_path,
    })
}

async fn canonical_cover_path(paths: &CoverPaths) -> HttpResult<PathBuf> {
    let canonical_root = canonical_or_not_found(&paths.plugin_root).await?;
    let canonical_cover = canonical_or_not_found(&paths.cover_path).await?;
    ensure_inside_root(&canonical_root, &canonical_cover)?;
    Ok(canonical_cover)
}

async fn read_cover_bytes(path: &PathBuf) -> HttpResult<Vec<u8>> {
    ensure_cover_size(path).await?;
    tokio::fs::read(path)
        .await
        .map_err(|_| cover_read_failed_response())
}

async fn ensure_existing_non_symlink(path: &PathBuf) -> HttpResult<()> {
    let metadata = metadata_or_not_found(path).await?;
    ensure_not_symlink(&metadata)
}

async fn ensure_cover_file(path: &PathBuf) -> HttpResult<()> {
    let metadata = metadata_or_not_found(path).await?;
    ensure_not_symlink(&metadata)?;
    ensure_file(&metadata)
}

async fn metadata_or_not_found(path: &PathBuf) -> HttpResult<std::fs::Metadata> {
    tokio::fs::symlink_metadata(path)
        .await
        .map_err(|_| cover_not_found_response())
}

async fn canonical_or_not_found(path: &PathBuf) -> HttpResult<PathBuf> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|_| cover_not_found_response())
}

async fn ensure_cover_size(path: &PathBuf) -> HttpResult<()> {
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|_| cover_read_failed_response())?
        .len() as usize;
    if size > MAX_COVER_SIZE {
        return Err(cover_too_large_response());
    }
    Ok(())
}

fn ensure_not_symlink(metadata: &std::fs::Metadata) -> HttpResult<()> {
    if metadata.file_type().is_symlink() {
        return Err(cover_forbidden_response());
    }
    Ok(())
}

fn ensure_file(metadata: &std::fs::Metadata) -> HttpResult<()> {
    if !metadata.is_file() {
        return Err(cover_not_found_response());
    }
    Ok(())
}

fn ensure_inside_root(root: &PathBuf, path: &PathBuf) -> HttpResult<()> {
    if !path.starts_with(root) {
        return Err(cover_forbidden_response());
    }
    Ok(())
}

fn cover_png_response(data: Vec<u8>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], data).into_response()
}

fn cover_not_found_response() -> Response {
    (StatusCode::NOT_FOUND, "Cover not found").into_response()
}

fn cover_forbidden_response() -> Response {
    (StatusCode::FORBIDDEN, "Invalid cover path").into_response()
}

fn cover_too_large_response() -> Response {
    (StatusCode::PAYLOAD_TOO_LARGE, "Cover image too large").into_response()
}

fn cover_read_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read cover").into_response()
}
