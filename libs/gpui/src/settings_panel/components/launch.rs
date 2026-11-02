use gpui::prelude::*;
use gpui::{div, img, px, rgb, rgba, App, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use super::RowGround;
use crate::kit::Kit;
use crate::pictures;
use crate::text::TextStyled;

const LAUNCH_MARK_SIZE: f32 = 14.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    tag: Option<SharedString>,
    app: Option<SharedString>,
}

impl Launch {
    pub fn file(file_name: &str, app: Option<String>) -> Self {
        Self {
            tag: extension_tag(file_name).map(SharedString::from),
            app: app.map(SharedString::from),
        }
    }

    pub fn link(host: impl Into<SharedString>) -> Self {
        Self {
            tag: None,
            app: Some(host.into()),
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LaunchTones {
    tag: u32,
    app: u32,
    mark: u32,
}

fn launch_tones(row: RowGround, kit: Kit) -> LaunchTones {
    let ground = row.rest(kit);
    match row {
        RowGround::Pane => LaunchTones {
            tag: ground.faint,
            app: ground.soft,
            mark: ground.soft,
        },
        RowGround::Band => LaunchTones {
            tag: ground.soft,
            app: ground.ink,
            mark: ground.ink,
        },
    }
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
        let tones = launch_tones(self.row, self.kit);
        let ground = self.row.rest(self.kit);
        let size_px = (LAUNCH_MARK_SIZE * window.scale_factor()).round() as u32;
        let mark = pictures::open_outside(tones.mark, size_px)
            .map(|image| img(image).w(px(LAUNCH_MARK_SIZE)).h(px(LAUNCH_MARK_SIZE)));
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .h(px(qol_theme::HEIGHT_INLINE))
            .px(px(qol_theme::SPACE_INSET))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .bg(rgba(ground.well.packed()))
            .children(self.launch.tag.map(|tag| {
                div()
                    .flex_none()
                    .text(TextStyle::Key)
                    .text_color(rgb(tones.tag))
                    .child(tag)
            }))
            .children(self.launch.app.map(|app| {
                div()
                    .flex_none()
                    .text(TextStyle::Value)
                    .text_color(rgb(tones.app))
                    .child(app)
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
    use super::{extension_tag, launch_tones, Launch, LaunchTones, RowGround};
    use crate::kit::Kit;
    use crate::theme::{ThemeMode, DARK_SYSTEM};

    #[test]
    fn a_file_tags_its_extension_and_names_its_app() {
        let launch = Launch::file(
            "20261102-152119-conflict.json",
            Some("Visual Studio Code".to_string()),
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
        assert_eq!(Launch::file("notes", None), Launch::default());
    }

    #[test]
    fn a_link_shows_its_host() {
        let launch = Launch::link("github.com");
        assert_eq!(launch.tag(), None);
        assert_eq!(launch.app().map(|app| app.as_ref()), Some("github.com"));
    }

    #[test]
    fn launch_tones_follow_the_row_ground() {
        let kit = Kit::new(ThemeMode::Dark, DARK_SYSTEM.with_accent(0x8a93f7));
        let pane = kit.grounds.pane;
        let band = kit.grounds.band;
        assert_eq!(
            launch_tones(RowGround::Pane, kit),
            LaunchTones {
                tag: pane.faint,
                app: pane.soft,
                mark: pane.soft,
            }
        );
        assert_eq!(
            launch_tones(RowGround::Band, kit),
            LaunchTones {
                tag: band.soft,
                app: band.ink,
                mark: band.ink,
            }
        );
    }
}
