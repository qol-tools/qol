use std::path::{Path, PathBuf};
use axum::{
    Router,
    extract::Path as AxumPath,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};

pub fn router(plugins_dir: PathBuf) -> Router {
    Router::new()
        .route("/:plugin_id", get(serve_plugin_index))
        .route("/:plugin_id/", get(serve_plugin_index))
        .route("/:plugin_id/*path", get(serve_plugin_file))
        .with_state(plugins_dir)
}

async fn serve_plugin_index(
    AxumPath(plugin_id): AxumPath<String>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
) -> Response {
    serve_file(&plugins_dir, &plugin_id, "index.html").await
}

async fn serve_plugin_file(
    AxumPath((plugin_id, path)): AxumPath<(String, String)>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
) -> Response {
    serve_file(&plugins_dir, &plugin_id, &path).await
}

async fn serve_file(plugins_dir: &Path, plugin_id: &str, file_path: &str) -> Response {
    let ui_path = plugins_dir.join(plugin_id).join("ui").join(file_path);
    log::debug!("Serving plugin file: {:?}", ui_path);

    if !ui_path.exists() {
        log::warn!("Plugin UI file not found: {:?}", ui_path);
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    }

    if !is_safe_path(&ui_path, plugins_dir) {
        log::warn!("Plugin UI path not safe: {:?}", ui_path);
        return (StatusCode::FORBIDDEN, "Access denied").into_response();
    }

    match tokio::fs::read(&ui_path).await {
        Ok(contents) => {
            let mime = guess_mime(&ui_path);
            log::debug!("Serving {:?} as {}", ui_path, mime);
            ([(header::CONTENT_TYPE, mime)], contents).into_response()
        }
        Err(e) => {
            log::error!("Failed to read plugin UI file {:?}: {}", ui_path, e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response()
        }
    }
}

fn is_safe_path(requested: &Path, base: &Path) -> bool {
    match (requested.canonicalize(), base.canonicalize()) {
        (Ok(req), Ok(base)) => req.starts_with(base),
        _ => false,
    }
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
    fn guess_mime_returns_correct_type_for_html() {
        // Arrange
        let path = PathBuf::from("index.html");

        // Act
        let mime = guess_mime(&path);

        // Assert
        assert_eq!(mime, "text/html; charset=utf-8");
    }

    #[test]
    fn guess_mime_returns_correct_type_for_css() {
        // Arrange
        let path = PathBuf::from("style.css");

        // Act
        let mime = guess_mime(&path);

        // Assert
        assert_eq!(mime, "text/css; charset=utf-8");
    }

    #[test]
    fn guess_mime_returns_correct_type_for_javascript() {
        // Arrange
        let path = PathBuf::from("app.js");

        // Act
        let mime = guess_mime(&path);

        // Assert
        assert_eq!(mime, "application/javascript; charset=utf-8");
    }

    #[test]
    fn guess_mime_returns_octet_stream_for_unknown() {
        // Arrange
        let path = PathBuf::from("data.bin");

        // Act
        let mime = guess_mime(&path);

        // Assert
        assert_eq!(mime, "application/octet-stream");
    }
}

