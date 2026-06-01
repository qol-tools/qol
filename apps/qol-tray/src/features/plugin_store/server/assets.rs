use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::IntoResponse,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "ui/"]
struct UiAssets;

pub(super) async fn serve_embedded(Path(path): Path<String>) -> impl IntoResponse {
    serve_embedded_file(&path)
}

pub(super) async fn serve_embedded_index() -> impl IntoResponse {
    let dev = super::boot::current_dev().await;
    match UiAssets::get("index.html") {
        Some(content) => {
            let html = String::from_utf8_lossy(&content.data);
            let injected = html.replacen(
                super::boot::BOOT_PLACEHOLDER,
                &format!("window.__QOL_BOOT__ = {};", super::boot::boot_json(dev)),
                1,
            );
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                injected,
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

pub(crate) fn serve_auto_config() -> impl IntoResponse {
    serve_embedded_file("auto-config.html")
}

fn mime_for_path(path: &str) -> &'static str {
    const MIME_MAP: &[(&str, &str)] = &[
        (".html", "text/html"),
        (".css", "text/css"),
        (".js", "application/javascript"),
        (".png", "image/png"),
        (".svg", "image/svg+xml"),
        (".wasm", "application/wasm"),
    ];
    MIME_MAP
        .iter()
        .find(|(ext, _)| path.ends_with(ext))
        .map(|(_, mime)| *mime)
        .unwrap_or("application/octet-stream")
}

#[cfg(test)]
pub(super) fn index_html_for_test() -> String {
    let content = UiAssets::get("index.html").expect("index.html embedded");
    String::from_utf8_lossy(&content.data).into_owned()
}

fn serve_embedded_file(path: &str) -> impl IntoResponse {
    let mime = mime_for_path(path);

    match UiAssets::get(path) {
        Some(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime)],
            content.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}
