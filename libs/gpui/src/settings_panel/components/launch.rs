use gpui::prelude::*;
use gpui::{div, img, px, rgb, rgba, App, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use super::{settings_description, RowGround};
use crate::kit::{Kit, KEY_CHIP_HEIGHT};
use crate::pictures;
use crate::text::TextStyled;

const LAUNCH_MARK_SIZE: f32 = 14.0;
const LAUNCH_SEPARATOR: &str = " · ";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    file: Option<SharedString>,
    tag: Option<SharedString>,
    app: Option<SharedString>,
}

impl Launch {
    pub fn file(file_name: &str, app: Option<String>) -> Self {
        Self {
            file: Some(SharedString::from(file_name.to_string())),
            tag: extension_tag(file_name).map(SharedString::from),
            app: app.map(SharedString::from),
        }
    }

    pub fn link() -> Self {
        Self::default()
    }

    pub fn file_name(&self) -> Option<&SharedString> {
        self.file.as_ref()
    }

    pub fn tag(&self) -> Option<&SharedString> {
        self.tag.as_ref()
    }

    pub fn app(&self) -> Option<&SharedString> {
        self.app.as_ref()
    }
}

fn extension_tag(file_name: &str) -> Option<String> {
    let extension = std::path::Path::new(file_name).extension()?.to_str()?;
    (!extension.is_empty()).then(|| extension.to_uppercase())
}

fn launch_ink(row: RowGround, kit: Kit) -> u32 {
    let ground = row.rest(kit);
    match row {
        RowGround::Pane => ground.soft,
        RowGround::Band => ground.ink,
    }
}

fn launch_detail(launch: &Launch, detail: &str) -> String {
    let mut text = String::new();
    if !detail.is_empty() {
        text.push_str(LAUNCH_SEPARATOR);
        text.push_str(detail);
    }
    if let Some(app) = &launch.app {
        text.push_str(LAUNCH_SEPARATOR);
        text.push_str("opens in ");
        text.push_str(app);
    }
    text
}

pub fn settings_launch_description(
    launch: &Launch,
    detail: &str,
    row: RowGround,
    kit: Kit,
) -> gpui::Div {
    div()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .overflow_hidden()
        .children(launch.file.clone().map(|file| {
            div()
                .flex_none()
                .text(TextStyle::Key)
                .text_color(rgb(launch_ink(row, kit)))
                .child(file)
        }))
        .child(
            settings_description(launch_detail(launch, detail), row, kit)
                .min_w_0()
                .truncate(),
        )
}

#[derive(IntoElement)]
pub(super) struct SettingsLaunch {
    launch: Launch,
    row: RowGround,
    kit: Kit,
}

impl SettingsLaunch {
    pub(super) fn new(launch: Launch, row: RowGround, kit: Kit) -> Self {
        Self { launch, row, kit }
    }
}

impl RenderOnce for SettingsLaunch {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ink = launch_ink(self.row, self.kit);
        let ground = self.row.rest(self.kit);
        let size_px = (LAUNCH_MARK_SIZE * window.scale_factor()).round() as u32;
        let mark = pictures::open_outside(ink, size_px)
            .map(|image| img(image).w(px(LAUNCH_MARK_SIZE)).h(px(LAUNCH_MARK_SIZE)));
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .h(px(qol_theme::HEIGHT_INLINE))
            .children(self.launch.tag.map(|tag| {
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .h(px(KEY_CHIP_HEIGHT))
                    .px(px(qol_theme::SPACE_SNUG))
                    .rounded(px(qol_theme::RADIUS_TIGHT))
                    .bg(rgba(ground.well.packed()))
                    .text(TextStyle::Key)
                    .text_color(rgb(ink))
                    .child(tag)
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(LAUNCH_MARK_SIZE))
                    .h(px(LAUNCH_MARK_SIZE))
                    .children(mark),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{extension_tag, launch_detail, launch_ink, Launch, RowGround};
    use crate::kit::Kit;
    use crate::theme::{ThemeMode, DARK_SYSTEM};

    #[test]
    fn a_file_tags_its_extension_and_names_its_app() {
        let launch = Launch::file(
            "20261102-152119-conflict.json",
            Some("Visual Studio Code".to_string()),
        );
        assert_eq!(
            launch.file_name().map(|file| file.as_ref()),
            Some("20261102-152119-conflict.json")
        );
        assert_eq!(launch.tag().map(|tag| tag.as_ref()), Some("JSON"));
        assert_eq!(
            launch.app().map(|app| app.as_ref()),
            Some("Visual Studio Code")
        );
    }

    #[test]
    fn a_file_without_an_extension_has_no_tag() {
        assert_eq!(extension_tag("README"), None);
        assert_eq!(extension_tag("archive.tar.gz").as_deref(), Some("GZ"));
        assert_eq!(Launch::file("notes", None).tag(), None);
    }

    #[test]
    fn a_link_carries_only_the_mark() {
        assert_eq!(Launch::link(), Launch::default());
    }

    #[test]
    fn the_description_follows_the_file_name_and_names_the_app() {
        let named = Launch::file("a.json", Some("Visual Studio Code".to_string()));
        let unnamed = Launch::file("a.json", None);
        let cases = [
            (&named, "Saved", " · Saved · opens in Visual Studio Code"),
            (&named, "", " · opens in Visual Studio Code"),
            (&unnamed, "Saved", " · Saved"),
            (&unnamed, "", ""),
        ];
        for (launch, detail, expected) in cases {
            assert_eq!(launch_detail(launch, detail), expected, "detail {detail:?}");
        }
    }

    #[test]
    fn the_tag_and_the_mark_follow_the_row_ground() {
        let kit = Kit::new(ThemeMode::Dark, DARK_SYSTEM.with_accent(0x8a93f7));
        assert_eq!(launch_ink(RowGround::Pane, kit), kit.grounds.pane.soft);
        assert_eq!(launch_ink(RowGround::Band, kit), kit.grounds.band.ink);
    }
}
