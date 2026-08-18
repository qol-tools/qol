use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::IntoResponse,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "ui/"]
struct UiAssets;

#[derive(Embed)]
#[folder = "../../libs/qol-config/js/"]
struct QolConfigAssets;

const QOL_CONFIG_ASSET_PREFIX: &str = "libs/qol-config/js/";

pub(super) const AUTH_FRAGMENT_KEY_PLACEHOLDER: &str =
    "window.__QOL_AUTH_FRAGMENT_KEY__ = null; /* QOL_AUTH_FRAGMENT_KEY_INJECT */";

pub(super) async fn serve_embedded(Path(path): Path<String>) -> impl IntoResponse {
    serve_embedded_file(&path)
}

pub(super) async fn serve_embedded_index() -> impl IntoResponse {
    let dev = super::boot::current_dev().await;
    match UiAssets::get("index.html") {
        Some(content) => {
            let html = String::from_utf8_lossy(&content.data);
            let injected = html
                .replacen(
                    super::boot::BOOT_PLACEHOLDER,
                    &format!("window.__QOL_BOOT__ = {};", super::boot::boot_json(dev)),
                    1,
                )
                .replacen(
                    AUTH_FRAGMENT_KEY_PLACEHOLDER,
                    &format!(
                        "window.__QOL_AUTH_FRAGMENT_KEY__ = '{}';",
                        qol_conventions::HTTP_AUTH_FRAGMENT_KEY
                    ),
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

    match embedded_file(path) {
        Some(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime)],
            content.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

fn embedded_file(path: &str) -> Option<rust_embed::EmbeddedFile> {
    if let Some(config_path) = path.strip_prefix(QOL_CONFIG_ASSET_PREFIX) {
        return QolConfigAssets::get(config_path);
    }
    UiAssets::get(path)
}

#[cfg(test)]
mod tests {
    use super::{embedded_file, QolConfigAssets, UiAssets, QOL_CONFIG_ASSET_PREFIX};

    #[test]
    fn embedded_ui_does_not_depend_on_remote_assets() {
        let violations = remote_asset_violations::<UiAssets>("")
            .into_iter()
            .chain(remote_asset_violations::<QolConfigAssets>(
                QOL_CONFIG_ASSET_PREFIX,
            ))
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "embedded UI assets must be self-contained; remote dependencies found in: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn shared_qol_config_assets_resolve_through_the_ui_server() {
        let asset = embedded_file("libs/qol-config/js/heuristics.js")
            .expect("shared qol-config asset embedded");
        let source = String::from_utf8_lossy(&asset.data);
        assert!(source.contains("export function prettyLabel"));
    }

    #[test]
    fn tray_ui_does_not_embed_copies_of_shared_qol_config_modules() {
        let duplicate_paths = [
            "auto-config/config-paths.js",
            "auto-config/display-rules.js",
            "auto-config/heuristics.js",
            "auto-config/normalized-config.js",
            "auto-config/object-array-form.js",
            "auto-config/object-array-renderer.js",
        ];

        for path in duplicate_paths {
            assert!(
                UiAssets::get(path).is_none(),
                "duplicate shared module: {path}"
            );
        }
    }

    fn remote_asset_violations<Assets: rust_embed::RustEmbed>(prefix: &str) -> Vec<String> {
        Assets::iter()
            .filter(|path| {
                path.ends_with(".css") || path.ends_with(".html") || path.ends_with(".js")
            })
            .filter_map(|path| {
                let asset = Assets::get(path.as_ref())?;
                let source = String::from_utf8_lossy(&asset.data);
                has_remote_asset_dependency(&source)
                    .then(|| format!("{prefix}{}", path.into_owned()))
            })
            .collect()
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
