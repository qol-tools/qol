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
    serve_embedded_file("index.html")
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
    ];
    MIME_MAP
        .iter()
        .find(|(ext, _)| path.ends_with(ext))
        .map(|(_, mime)| *mime)
        .unwrap_or("application/octet-stream")
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
