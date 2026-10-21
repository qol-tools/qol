use gpui::{div, img, px, AnyElement, App, IntoElement, ParentElement, RenderOnce, Styled, Window};
use qol_gpui::kit::kit;
use qol_gpui::theme::font_display;

use crate::listing::Entry;
use crate::ui::kind::{kind_of, tag_text, FileKind};

macro_rules! sized_assets {
    ($stem:literal) => {
        [
            include_str!(concat!("../../../assets/pictograms/", $stem, "-16.svg")),
            include_str!(concat!("../../../assets/pictograms/", $stem, "-32.svg")),
            include_str!(concat!("../../../assets/pictograms/", $stem, "-56.svg")),
        ]
    };
}

const FOLDER: &str = include_str!("../../../assets/pictograms/folder.svg");
const HOME: &str = include_str!("../../../assets/pictograms/home-16.svg");
const DRIVE: &str = include_str!("../../../assets/pictograms/drive-16.svg");
const SVG_END: &str = "</svg>";
const TAG_INK: u32 = 0x16171a;
const RAIL_SIZE: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PictogramSize {
    Small,
    Medium,
    Large,
}

impl PictogramSize {
    pub(super) fn px(self) -> f32 {
        match self {
            Self::Small => 16.0,
            Self::Medium => 32.0,
            Self::Large => 56.0,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Small => 0,
            Self::Medium => 1,
            Self::Large => 2,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum RailGlyph {
    Home,
    Folder,
    Drive,
}

pub(super) fn entry_pictogram(entry: &Entry, size: PictogramSize, selected: bool) -> AnyElement {
    if entry.opens_as_folder {
        return folder(size.px());
    }
    let kit = kit();
    let ink = if selected {
        kit.grounds.band.ink
    } else {
        kit.grounds.pane.soft
    };
    let kind = kind_of(&entry.display, entry.executable);
    let tag = tag_text(&entry.display, kind);
    Pictogram {
        name: format!("explorer-{}-{}-{tag}", kind.asset_stem(), size.px()),
        markup: tagged(outline(kind)[size.index()], kind, &tag),
        size: size.px(),
        ink,
    }
    .into_any_element()
}

pub(super) fn rail_pictogram(glyph: RailGlyph, selected: bool) -> AnyElement {
    let kit = kit();
    let ink = if selected {
        kit.grounds.band.ink
    } else {
        kit.grounds.rail.soft
    };
    let (name, markup) = match glyph {
        RailGlyph::Folder => return folder(RAIL_SIZE),
        RailGlyph::Home => ("explorer-home", HOME),
        RailGlyph::Drive => ("explorer-drive", DRIVE),
    };
    Pictogram {
        name: name.to_string(),
        markup: markup.to_string(),
        size: RAIL_SIZE,
        ink,
    }
    .into_any_element()
}

fn folder(size: f32) -> AnyElement {
    Pictogram {
        name: "explorer-folder".to_string(),
        markup: FOLDER.to_string(),
        size,
        ink: 0,
    }
    .into_any_element()
}

fn tagged(outline: &str, kind: FileKind, tag: &str) -> String {
    let end = outline.rfind(SVG_END).unwrap_or(outline.len());
    format!(
        r##"{}<rect x="2" y="14" width="20" height="6.5" rx="1" fill="#{:06x}" stroke="none"/><text x="12" y="18.9" fill="#{TAG_INK:06x}" stroke="none" font-family="{}" font-weight="600" font-size="5.6" text-anchor="middle">{tag}</text>{SVG_END}"##,
        &outline[..end],
        kind.tag_color(),
        font_display(),
    )
}

fn outline(kind: FileKind) -> [&'static str; 3] {
    match kind {
        FileKind::Text => sized_assets!("text"),
        FileKind::Document => sized_assets!("document"),
        FileKind::Sheet => sized_assets!("sheet"),
        FileKind::Markdown => sized_assets!("markdown"),
        FileKind::Log => sized_assets!("log"),
        FileKind::Config => sized_assets!("config"),
        FileKind::Data => sized_assets!("data"),
        FileKind::Book => sized_assets!("book"),
        FileKind::Slides => sized_assets!("slides"),
        FileKind::Image => sized_assets!("image"),
        FileKind::Video => sized_assets!("video"),
        FileKind::Audio => sized_assets!("audio"),
        FileKind::Font => sized_assets!("font"),
        FileKind::Code => sized_assets!("code"),
        FileKind::Script => sized_assets!("script"),
        FileKind::Web => sized_assets!("web"),
        FileKind::Binary => sized_assets!("binary"),
        FileKind::Archive => sized_assets!("archive"),
        FileKind::Package => sized_assets!("package"),
        FileKind::Disc => sized_assets!("disc"),
        FileKind::Database => sized_assets!("database"),
        FileKind::Key => sized_assets!("key"),
        FileKind::Link => sized_assets!("link"),
        FileKind::Unknown => sized_assets!("unknown"),
    }
}

#[derive(IntoElement)]
struct Pictogram {
    name: String,
    markup: String,
    size: f32,
    ink: u32,
}

impl RenderOnce for Pictogram {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size_px = (self.size * window.scale_factor()).round() as u32;
        let image = qol_gpui::pictures::icon(&self.name, &self.markup, self.ink, size_px);
        div()
            .flex_none()
            .size(px(self.size))
            .children(image.map(|image| img(image).size(px(self.size))))
    }
}

#[cfg(test)]
mod tests {
    use super::{outline, tagged, PictogramSize, DRIVE, FOLDER, HOME};
    use crate::ui::kind::{tag_text, FileKind};

    #[test]
    fn sizes_are_the_three_pictogram_stops() {
        let cases = [
            (PictogramSize::Small, 16.0),
            (PictogramSize::Medium, 32.0),
            (PictogramSize::Large, 56.0),
        ];
        for (size, px) in cases {
            assert_eq!(size.px(), px, "{size:?}");
        }
    }

    #[test]
    fn every_tagged_pictogram_rasterizes() {
        for kind in FileKind::ALL {
            for (index, markup) in outline(kind).into_iter().enumerate() {
                let source = tagged(markup, kind, &tag_text("file", kind));
                let name = format!("explorer-test-{}-{index}", kind.asset_stem());
                let image = qol_gpui::pictures::icon(&name, &source, 0x8b8880, 32);
                assert!(image.is_some(), "{kind:?} {index}");
            }
        }
    }

    #[test]
    fn rail_and_folder_assets_rasterize() {
        for (name, markup) in [("folder", FOLDER), ("home", HOME), ("drive", DRIVE)] {
            let name = format!("explorer-test-{name}");
            assert!(
                qol_gpui::pictures::icon(&name, markup, 0x8b8880, 16).is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn outlines_follow_the_ink_and_the_tag_keeps_its_colour() {
        for kind in FileKind::ALL {
            for markup in outline(kind) {
                assert!(markup.contains("currentColor"), "{kind:?}");
                assert!(!markup.contains("black"), "{kind:?}");
            }
            let source = tagged(outline(kind)[0], kind, "TAG");
            assert!(
                source.contains(&format!("#{:06x}", kind.tag_color())),
                "{kind:?}"
            );
            assert!(source.ends_with("</svg>"), "{kind:?}");
        }
    }
}
