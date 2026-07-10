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

#[cfg(test)]
mod tests {
    use super::UiAssets;

    #[test]
    fn embedded_ui_does_not_depend_on_remote_assets() {
        let violations = UiAssets::iter()
            .filter(|path| {
                path.ends_with(".css") || path.ends_with(".html") || path.ends_with(".js")
            })
            .filter_map(|path| {
                let asset = UiAssets::get(path.as_ref())?;
                let source = String::from_utf8_lossy(&asset.data);
                has_remote_asset_dependency(&source).then(|| path.into_owned())
            })
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "embedded UI assets must be self-contained; remote dependencies found in: {}",
            violations.join(", ")
        );
    }

    fn has_remote_asset_dependency(source: &str) -> bool {
        let compact = source
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>();

        const LOAD_PREFIXES: &[&str] = &[
            "from'",
            "from\"",
            "import('",
            "import(\"",
            "import'",
            "import\"",
            "url('",
            "url(\"",
            "url(",
        ];
        const REMOTE_LOCATIONS: &[&str] = &["http://", "https://", "//"];

        REMOTE_LOCATIONS.iter().any(|location| {
            LOAD_PREFIXES
                .iter()
                .any(|prefix| compact.contains(&format!("{prefix}{location}")))
        }) || compact.split('<').any(|fragment| {
            let tag = fragment.split_once('>').map_or(fragment, |(tag, _)| tag);
            let remote = REMOTE_LOCATIONS
                .iter()
                .any(|location| tag.contains(location));
            remote
                && ((tag.starts_with("script") && tag.contains("src="))
                    || (tag.starts_with("link")
                        && tag.contains("stylesheet")
                        && tag.contains("href=")))
        })
    }
}
