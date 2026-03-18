use axum::{
    extract::Path as AxumPath,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::path::{Path, PathBuf};

use crate::plugins::paths as plugin_paths;

pub(super) fn router(plugins_dir: PathBuf) -> Router {
    Router::new()
        .route("/{plugin_id}", get(serve_plugin_index))
        .route("/{plugin_id}/", get(serve_plugin_index))
        .route("/{plugin_id}/{*path}", get(serve_plugin_file))
        .with_state(plugins_dir)
}

async fn serve_plugin_index(
    AxumPath(plugin_id): AxumPath<String>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
) -> Response {
    let plugin_root = super::plugin_paths::resolve_plugin_root(&plugins_dir, &plugin_id);
    if !plugin_paths::has_custom_ui(&plugin_root) && !plugin_paths::has_config(&plugin_root) {
        return (StatusCode::NOT_FOUND, "No settings UI available").into_response();
    }
    super::server::assets::serve_auto_config().into_response()
}

async fn serve_plugin_file(
    AxumPath((plugin_id, path)): AxumPath<(String, String)>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
) -> Response {
    serve_file(&plugins_dir, &plugin_id, &path).await
}

async fn serve_file(plugins_dir: &Path, plugin_id: &str, file_path: &str) -> Response {
    let ui_path = match resolve_safe_ui_file(plugins_dir, plugin_id, file_path).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    log::debug!("Serving plugin file: {:?}", ui_path);

    let contents = match tokio::fs::read(&ui_path).await {
        Ok(contents) => contents,
        Err(e) => {
            log::error!("Failed to read plugin UI file {:?}: {}", ui_path, e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response();
        }
    };

    let mime = guess_mime(&ui_path);
    log::debug!("Serving {:?} as {}", ui_path, mime);
    ([(header::CONTENT_TYPE, mime)], contents).into_response()
}

async fn canonicalize_or_not_found(path: &Path) -> Result<PathBuf, Response> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "File not found").into_response())
}

async fn validate_dir_entry(path: &Path) -> Result<(), Response> {
    let meta = match tokio::fs::symlink_metadata(path).await {
        Ok(meta) => meta,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }
    Ok(())
}

async fn validate_file_entry(path: &Path) -> Result<(), Response> {
    let meta = match tokio::fs::symlink_metadata(path).await {
        Ok(meta) => meta,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
    if meta.file_type().is_symlink() {
        log::warn!("Symlink rejected: {:?}", path);
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }
    if !meta.is_file() {
        return Err((StatusCode::NOT_FOUND, "File not found").into_response());
    }
    Ok(())
}

async fn verify_path_chain(
    canonical_plugin_root: &Path,
    ui_root: &Path,
    ui_path: &Path,
) -> Result<PathBuf, Response> {
    let canonical_ui_root = canonicalize_or_not_found(ui_root).await?;
    if !canonical_ui_root.starts_with(canonical_plugin_root) {
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }
    let canonical_ui_path = canonicalize_or_not_found(ui_path).await?;
    if !canonical_ui_path.starts_with(&canonical_ui_root) {
        log::warn!(
            "Path traversal attempt: {:?} escapes {:?}",
            canonical_ui_path,
            canonical_ui_root
        );
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }
    Ok(canonical_ui_path)
}

async fn resolve_safe_ui_file(
    plugins_dir: &Path,
    plugin_id: &str,
    file_path: &str,
) -> Result<PathBuf, Response> {
    if !super::validation::is_safe_plugin_id(plugin_id) || !is_safe_subpath(file_path) {
        log::warn!(
            "Unsafe path: plugin_id={}, file_path={}",
            plugin_id,
            file_path
        );
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }
    let plugin_root = super::plugin_paths::resolve_plugin_root(plugins_dir, plugin_id);
    let ui_root = plugin_root.join("ui");
    let ui_path = ui_root.join(file_path);
    validate_dir_entry(&plugin_root).await?;
    validate_dir_entry(&ui_root).await?;
    validate_file_entry(&ui_path).await?;
    let Some(canonical_plugin_root) = verify_plugin_root_allowed(plugins_dir, plugin_id) else {
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    };
    verify_path_chain(&canonical_plugin_root, &ui_root, &ui_path).await
}

fn verify_plugin_root_allowed(plugins_dir: &Path, plugin_id: &str) -> Option<PathBuf> {
    super::plugin_paths::canonical_plugin_root(plugins_dir, plugin_id)
}

fn is_safe_subpath(path: &str) -> bool {
    !path.contains("..")
        && !path.contains('\0')
        && !path.starts_with('/')
        && !path.starts_with('\\')
}

fn guess_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_subpath_validation() {
        let cases = [
            ("index.html", true),
            ("css/style.css", true),
            ("a/b/c/d.js", true),
            ("file.min.js", true),
            ("../secret.txt", false),
            ("foo/../bar", false),
            ("foo/bar/../baz", false),
            ("....//....//etc", false),
            ("/etc/passwd", false),
            ("\\windows\\system32", false),
            ("..\\secret", false),
            ("..", false),
            ("a/../../b", false),
            ("valid/..invalid", false),
            ("file\0.txt", false),
        ];

        for (path, expected) in cases {
            assert_eq!(is_safe_subpath(path), expected, "path: {:?}", path);
        }
    }

    #[test]
    fn guess_mime_returns_correct_types() {
        let cases = [
            ("index.html", "text/html; charset=utf-8"),
            ("style.css", "text/css; charset=utf-8"),
            ("app.js", "application/javascript; charset=utf-8"),
            ("data.json", "application/json"),
            ("image.png", "image/png"),
            ("data.bin", "application/octet-stream"),
        ];

        for (filename, expected) in cases {
            let path = PathBuf::from(filename);
            assert_eq!(guess_mime(&path), expected, "file: {}", filename);
        }
    }
}
