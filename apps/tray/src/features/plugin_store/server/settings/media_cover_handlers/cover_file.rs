use axum::{http::StatusCode, response::IntoResponse, response::Response};
use std::path::{Path, PathBuf};

use super::super::super::types::MAX_COVER_SIZE;
use crate::plugins::paths as plugin_paths;

struct CoverPaths {
    plugin_root: PathBuf,
    cover_path: PathBuf,
}

pub(super) async fn load_cover_bytes(
    base_dir: &Path,
    plugin_id: &str,
) -> Result<Vec<u8>, Box<Response>> {
    let paths = cover_paths(base_dir, plugin_id).await?;
    let canonical_cover = canonical_cover_path(&paths).await?;
    ensure_cover_size(&canonical_cover).await?;
    tokio::fs::read(&canonical_cover)
        .await
        .map_err(|_| Box::new(cover_read_failed_response()))
}

async fn cover_paths(base_dir: &Path, plugin_id: &str) -> Result<CoverPaths, Box<Response>> {
    let plugin_root = plugin_paths::resolve_plugin_root_from_plugins_dir(base_dir, plugin_id);
    ensure_existing_non_symlink(&plugin_root).await?;
    let cover_path = plugin_root.join("cover.png");
    ensure_cover_file(&cover_path).await?;
    Ok(CoverPaths {
        plugin_root,
        cover_path,
    })
}

async fn canonical_cover_path(paths: &CoverPaths) -> Result<PathBuf, Box<Response>> {
    let canonical_root = canonical_or_not_found(&paths.plugin_root).await?;
    let canonical_cover = canonical_or_not_found(&paths.cover_path).await?;
    if !canonical_cover.starts_with(&canonical_root) {
        return Err(Box::new(cover_forbidden_response()));
    }
    Ok(canonical_cover)
}

async fn ensure_existing_non_symlink(path: &Path) -> Result<(), Box<Response>> {
    let metadata = metadata_or_not_found(path).await?;
    ensure_not_symlink(&metadata)
}

async fn ensure_cover_file(path: &Path) -> Result<(), Box<Response>> {
    let metadata = metadata_or_not_found(path).await?;
    ensure_not_symlink(&metadata)?;
    ensure_file(&metadata)
}

async fn metadata_or_not_found(path: &Path) -> Result<std::fs::Metadata, Box<Response>> {
    tokio::fs::symlink_metadata(path)
        .await
        .map_err(|_| Box::new(cover_not_found_response()))
}

async fn canonical_or_not_found(path: &Path) -> Result<PathBuf, Box<Response>> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|_| Box::new(cover_not_found_response()))
}

async fn ensure_cover_size(path: &Path) -> Result<(), Box<Response>> {
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|_| Box::new(cover_read_failed_response()))?
        .len() as usize;
    if size > MAX_COVER_SIZE {
        return Err(Box::new(cover_too_large_response()));
    }
    Ok(())
}

fn ensure_not_symlink(metadata: &std::fs::Metadata) -> Result<(), Box<Response>> {
    if metadata.file_type().is_symlink() {
        return Err(Box::new(cover_forbidden_response()));
    }
    Ok(())
}

fn ensure_file(metadata: &std::fs::Metadata) -> Result<(), Box<Response>> {
    if !metadata.is_file() {
        return Err(Box::new(cover_not_found_response()));
    }
    Ok(())
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
