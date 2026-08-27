use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Context as _;
use gpui::*;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "mov", "avi", "m4v"];
const SLOT_WIDTH: u32 = 72;
const SLOT_HEIGHT: u32 = 68;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    Image,
    Video,
    File,
}

impl ArtifactKind {
    pub fn of(path: &Path) -> Self {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some(extension) if IMAGE_EXTENSIONS.contains(&extension) => Self::Image,
            Some(extension) if VIDEO_EXTENSIONS.contains(&extension) => Self::Video,
            _ => Self::File,
        }
    }
}

/// How the 72x68 preview slot of a slab row draws one saved file.
pub trait ArtifactPreview {
    fn render(&self, tone_color: u32) -> AnyElement;
}

pub struct ImagePreview {
    path: Arc<Path>,
    dimensions: Option<(u32, u32)>,
}

impl ImagePreview {
    pub fn new(path: Arc<Path>) -> Self {
        let dimensions = image::ImageReader::open(&path)
            .ok()
            .and_then(|reader| reader.into_dimensions().ok());
        Self { path, dimensions }
    }
}

fn object_fit_for(dimensions: Option<(u32, u32)>) -> ObjectFit {
    match dimensions {
        Some((width, height)) if width < SLOT_WIDTH || height < SLOT_HEIGHT => ObjectFit::ScaleDown,
        _ => ObjectFit::Cover,
    }
}

impl ArtifactPreview for ImagePreview {
    fn render(&self, _tone_color: u32) -> AnyElement {
        img(self.path.clone())
            .w_full()
            .h_full()
            .object_fit(object_fit_for(self.dimensions))
            .into_any_element()
    }
}

pub struct ExtensionBadge {
    label: SharedString,
}

impl ExtensionBadge {
    pub fn for_path(path: &Path) -> Self {
        let label = path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| !extension.is_empty())
            .map(str::to_ascii_uppercase)
            .unwrap_or_else(|| "FILE".to_string());
        Self {
            label: label.into(),
        }
    }
}

impl ArtifactPreview for ExtensionBadge {
    fn render(&self, tone_color: u32) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(crate::kit::alpha(tone_color, 51)))
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_NANO))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(tone_color))
                    .child(self.label.clone()),
            )
            .into_any_element()
    }
}

pub fn preview_for(path: &Path) -> Rc<dyn ArtifactPreview> {
    match ArtifactKind::of(path) {
        ArtifactKind::Image => Rc::new(ImagePreview::new(path.into())),
        ArtifactKind::Video | ArtifactKind::File => Rc::new(ExtensionBadge::for_path(path)),
    }
}

pub fn open_artifact(path: &Path) -> anyhow::Result<()> {
    qol_apps::desktop_integration::open_with_default_app(path)
        .with_context(|| format!("could not open {}", file_name(path)))
}

pub fn reveal_artifact(path: &Path) -> anyhow::Result<()> {
    qol_apps::desktop_integration::reveal_in_file_manager(path)
        .with_context(|| format!("could not open the folder of {}", file_name(path)))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{object_fit_for, ArtifactKind, ExtensionBadge};
    use gpui::ObjectFit;
    use std::path::Path;

    #[test]
    fn kind_follows_the_extension_case_insensitively() {
        let cases = [
            ("shot.png", ArtifactKind::Image),
            ("photo.JPG", ArtifactKind::Image),
            ("clip.mp4", ArtifactKind::Video),
            ("clip.MKV", ArtifactKind::Video),
            ("notes.txt", ArtifactKind::File),
            ("archive.tar.gz", ArtifactKind::File),
            ("noext", ArtifactKind::File),
        ];
        for (name, expected) in cases {
            assert_eq!(ArtifactKind::of(Path::new(name)), expected, "{name}");
        }
    }

    #[test]
    fn badge_label_is_the_uppercase_extension_or_file() {
        let cases = [("clip.mp4", "MP4"), ("a.tar.gz", "GZ"), ("noext", "FILE")];
        for (name, expected) in cases {
            assert_eq!(
                ExtensionBadge::for_path(Path::new(name)).label.as_ref(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn object_fit_scales_down_only_below_the_slot() {
        assert!(matches!(object_fit_for(None), ObjectFit::Cover));
        assert!(matches!(
            object_fit_for(Some((71, 68))),
            ObjectFit::ScaleDown
        ));
        assert!(matches!(
            object_fit_for(Some((72, 67))),
            ObjectFit::ScaleDown
        ));
        assert!(matches!(object_fit_for(Some((72, 68))), ObjectFit::Cover));
        assert!(matches!(
            object_fit_for(Some((1920, 1080))),
            ObjectFit::Cover
        ));
    }
}
