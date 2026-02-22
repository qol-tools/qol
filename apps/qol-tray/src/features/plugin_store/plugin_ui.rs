use axum::{
    extract::Path as AxumPath,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
#[cfg(feature = "dev")]
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn router(plugins_dir: PathBuf) -> Router {
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
    let ui_path = match resolve_safe_ui_file(&plugins_dir, &plugin_id, "index.html").await {
        Ok(path) => path,
        Err(response) => return response,
    };

    let contents = match tokio::fs::read_to_string(&ui_path).await {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to read plugin UI: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response();
        }
    };

    let injected = inject_plugin_wrapper(&contents);
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        injected,
    )
        .into_response()
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
    let Some(pos) = insert_pos else {
        return html.to_string();
    };
    format!("{}{}{}", &html[..pos], content, &html[pos..])
}

fn find_body_tag_end(html: &str) -> Option<usize> {
    let lower = html.to_lowercase();
    let mut search_start = 0;

    while let Some(rel_pos) = lower[search_start..].find("<body") {
        let body_start = search_start + rel_pos;

        if !is_inside_comment(html, body_start) {
            let tag_end = find_tag_end(&html[body_start..])?;
            return Some(body_start + tag_end + 1);
        }

        search_start = body_start + 5;
    }

    None
}

fn find_tag_end(tag: &str) -> Option<usize> {
    let mut in_quote = None;

    for (i, c) in tag.char_indices() {
        match (c, in_quote) {
            ('"' | '\'', None) => in_quote = Some(c),
            (q, Some(open)) if q == open => in_quote = None,
            ('>', None) => return Some(i),
            _ => {}
        }
    }

    None
}

fn is_inside_comment(html: &str, pos: usize) -> bool {
    let before = &html[..pos];
    let comment_start = before.rfind("<!--");
    let comment_end = before.rfind("-->");

    match (comment_start, comment_end) {
        (Some(start), Some(end)) => start > end,
        (Some(_), None) => true,
        _ => false,
    }
}

fn inject_before_closing_body(html: &str, content: &str) -> String {
    let Some(pos) = html.rfind("</body>") else {
        return html.to_string();
    };
    format!("{}{}{}", &html[..pos], content, &html[pos..])
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

use crate::paths::is_safe_path_component;

async fn resolve_safe_ui_file(
    plugins_dir: &Path,
    plugin_id: &str,
    file_path: &str,
) -> Result<PathBuf, Response> {
    if !is_safe_path_component(plugin_id) || !is_safe_subpath(file_path) {
        log::warn!(
            "Unsafe path: plugin_id={}, file_path={}",
            plugin_id,
            file_path
        );
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }

    let plugin_root = resolve_plugin_root(plugins_dir, plugin_id);
    let ui_root = plugin_root.join("ui");
    let ui_path = ui_root.join(file_path);

    let plugin_meta = match tokio::fs::symlink_metadata(&plugin_root).await {
        Ok(meta) => meta,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
    if plugin_meta.file_type().is_symlink() || !plugin_meta.is_dir() {
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }

    let ui_meta = match tokio::fs::symlink_metadata(&ui_root).await {
        Ok(meta) => meta,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
    if ui_meta.file_type().is_symlink() || !ui_meta.is_dir() {
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }

    let file_meta = match tokio::fs::symlink_metadata(&ui_path).await {
        Ok(meta) => meta,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
    if file_meta.file_type().is_symlink() {
        log::warn!("Symlink rejected: {:?}", ui_path);
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }
    if !file_meta.is_file() {
        return Err((StatusCode::NOT_FOUND, "File not found").into_response());
    }

    let canonical_plugins_dir = match tokio::fs::canonicalize(plugins_dir).await {
        Ok(path) => path,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
    let canonical_plugin_root = match tokio::fs::canonicalize(&plugin_root).await {
        Ok(path) => path,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };

    let under_installed_root = canonical_plugin_root.starts_with(&canonical_plugins_dir);
    #[cfg(feature = "dev")]
    let under_dev_link_root = is_dev_link_target(plugin_id, &canonical_plugin_root);
    #[cfg(not(feature = "dev"))]
    let under_dev_link_root = false;

    if !under_installed_root && !under_dev_link_root {
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }

    let canonical_ui_root = match tokio::fs::canonicalize(&ui_root).await {
        Ok(path) => path,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
    if !canonical_ui_root.starts_with(&canonical_plugin_root) {
        return Err((StatusCode::FORBIDDEN, "Access denied").into_response());
    }

    let canonical_ui_path = match tokio::fs::canonicalize(&ui_path).await {
        Ok(path) => path,
        Err(_) => return Err((StatusCode::NOT_FOUND, "File not found").into_response()),
    };
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

fn resolve_plugin_root(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    #[cfg(feature = "dev")]
    if let Some(dev_path) = resolve_dev_link_path(plugin_id) {
        return dev_path;
    }

    let installed_root = plugins_dir.join(plugin_id);
    if installed_root.exists() {
        return installed_root;
    }

    installed_root
}

#[cfg(feature = "dev")]
fn resolve_dev_link_path(plugin_id: &str) -> Option<PathBuf> {
    let config_dir = crate::paths::shared_config_dir().ok()?;
    let dev_links_path = config_dir.join("dev-links.json");
    let content = std::fs::read_to_string(dev_links_path).ok()?;
    let links: HashMap<String, PathBuf> = serde_json::from_str(&content).ok()?;
    links.get(plugin_id).cloned()
}

#[cfg(feature = "dev")]
fn is_dev_link_target(plugin_id: &str, candidate: &Path) -> bool {
    let Some(dev_path) = resolve_dev_link_path(plugin_id) else {
        return false;
    };
    let Ok(canonical_dev_path) = std::fs::canonicalize(dev_path) else {
        return false;
    };
    candidate == canonical_dev_path
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

    #[test]
    fn find_body_tag_end_cases() {
        let cases = [
            ("<html><body>", Some(12)),
            ("<html><BODY>", Some(12)),
            ("<html><Body class='x'>", Some(22)),
            ("<!-- <body> --><body>", Some(21)),
            ("<!-- <body> -->", None),
            ("", None),
            ("<html><head></head></html>", None),
            ("<body data-x='a>b'>", Some(19)),
            ("<body data-x=\"a>b\">", Some(19)),
            ("<!--<body>--><body id='real'>", Some(29)),
            ("<body onclick=\"if(a>b){}\">", Some(26)),
        ];

        for (html, expected) in cases {
            assert_eq!(find_body_tag_end(html), expected, "html: {:?}", html);
        }
    }

    #[test]
    fn is_inside_comment_cases() {
        let cases = [
            ("<body>", 0, false),
            ("<!-- <body> -->", 5, true),
            ("<!-- --> <body>", 9, false),
            ("<!-- x --> <!-- <body>", 16, true),
            ("<!-- a --> <!-- b --> x", 22, false),
            ("<!-- unclosed", 5, true),
            ("text <!-- comment -->", 0, false),
        ];

        for (html, pos, expected) in cases {
            assert_eq!(
                is_inside_comment(html, pos),
                expected,
                "html: {:?}, pos: {}",
                html,
                pos
            );
        }
    }
}
