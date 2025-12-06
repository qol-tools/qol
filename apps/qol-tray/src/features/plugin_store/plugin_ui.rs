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
    let ui_path = plugins_dir.join(&plugin_id).join("ui").join("index.html");

    if !ui_path.exists() {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    }

    if !is_safe_path(&ui_path, &plugins_dir) {
        return (StatusCode::FORBIDDEN, "Access denied").into_response();
    }

    let contents = match tokio::fs::read_to_string(&ui_path).await {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to read plugin UI: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response();
        }
    };

    let injected = inject_plugin_wrapper(&contents);
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], injected).into_response()
}

fn inject_plugin_wrapper(html: &str) -> String {
    const NAV_HEADER: &str = r#"<div id="qol-plugin-nav" style="position:fixed;top:0;left:0;right:0;background:#1a1a1a;border-bottom:1px solid #333;padding:0.5rem 1rem;display:flex;align-items:center;gap:1rem;z-index:9999;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif">
<a href="/" style="color:#4a9eff;text-decoration:none;font-size:0.9rem">← Back</a>
</div>
<div style="height:2.5rem"></div>"#;

    const NAV_FOOTER: &str = r#"<div id="qol-plugin-footer" style="position:fixed;bottom:0;left:0;right:0;background:#1a1a1a;border-top:1px solid #333;padding:0.5rem;text-align:center;color:#666;font-size:0.85rem;z-index:9999;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif">
Esc back
</div>
<script>document.addEventListener('keydown',e=>{if(e.key==='Escape'){e.preventDefault();window.location.href='/';}})</script>"#;

    let with_header = inject_after_body_tag(html, NAV_HEADER);
    inject_before_closing_body(&with_header, NAV_FOOTER)
}

fn inject_after_body_tag(html: &str, content: &str) -> String {
    let insert_pos = find_body_tag_end(html);
    let Some(pos) = insert_pos else { return html.to_string() };
    format!("{}{}{}", &html[..pos], content, &html[pos..])
}

fn find_body_tag_end(html: &str) -> Option<usize> {
    let body_start = html.find("<body")?;
    let tag_end = html[body_start..].find('>')?;
    Some(body_start + tag_end + 1)
}

fn inject_before_closing_body(html: &str, content: &str) -> String {
    let Some(pos) = html.rfind("</body>") else { return html.to_string() };
    format!("{}{}{}", &html[..pos], content, &html[pos..])
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

