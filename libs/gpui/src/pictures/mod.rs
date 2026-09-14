mod letters;
mod library;
mod raster;
mod svg;

pub use letters::letters_for;
pub use raster::{image, tick};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PictureContext {
    pub mode: qol_theme::ThemeMode,
    pub bone: qol_theme::DesktopThemePreview,
    pub slate: qol_theme::DesktopThemePreview,
    pub web_slate: qol_theme::WebThemePreview,
    pub web_midnight: qol_theme::WebThemePreview,
}

impl PictureContext {
    pub fn for_accent(mode: qol_theme::ThemeMode, accent_key: &str) -> Self {
        Self {
            mode,
            bone: qol_theme::desktop_theme_preview(qol_theme::ThemeMode::Light, accent_key),
            slate: qol_theme::desktop_theme_preview(qol_theme::ThemeMode::Dark, accent_key),
            web_slate: qol_theme::web_theme_preview("slate").expect("slate web preview"),
            web_midnight: qol_theme::web_theme_preview("midnight").expect("midnight web preview"),
        }
    }
}

pub fn markup(spec: &str, context: &PictureContext) -> Option<String> {
    if !qol_config::contract::is_picture_spec(spec) {
        return None;
    }
    svg::reset_masks();
    library::markup_for(spec, context)
}
