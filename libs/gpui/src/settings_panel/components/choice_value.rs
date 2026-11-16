use gpui::prelude::*;
use gpui::{div, img, px, rgba, App, RenderOnce, Window};
use qol_theme::Mark;

use super::{
    RowGround, CHOICE_CHEVRON_HEIGHT, CHOICE_CHEVRON_HOVER_STEP, CHOICE_CHEVRON_REST_OPACITY,
    CHOICE_CHEVRON_WIDTH, CHOICE_PICTURE_HEIGHT, CHOICE_PICTURE_WIDTH, SETTINGS_ROW_GROUP,
    WELL_HEIGHT, WELL_MARK_SIZE, WELL_PICTURE_HEIGHT, WELL_PICTURE_WIDTH, WELL_WIDTH,
};
use crate::kit::Kit;
use crate::pictures::{self, PictureContext, Tone};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceArt {
    Picture(String),
    Stack { front: String, back: String },
    Mark(Mark),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ChoiceTones {
    line: u32,
    chevron: u32,
    chevron_opacity: f32,
    wakes_on_hover: bool,
}

fn choice_tones(row: RowGround, kit: Kit) -> ChoiceTones {
    match row {
        RowGround::Pane => ChoiceTones {
            line: kit.grounds.pane.soft,
            chevron: kit.grounds.pane.faint,
            chevron_opacity: CHOICE_CHEVRON_REST_OPACITY,
            wakes_on_hover: true,
        },
        RowGround::Band => ChoiceTones {
            line: kit.grounds.band.ink,
            chevron: kit.grounds.band.faint,
            chevron_opacity: 1.0,
            wakes_on_hover: false,
        },
    }
}

#[derive(IntoElement)]
pub struct SettingsChoiceArt {
    art: ChoiceArt,
    row: RowGround,
    context: PictureContext,
    kit: Kit,
    width: f32,
    height: f32,
}

impl SettingsChoiceArt {
    pub fn new(art: ChoiceArt, row: RowGround, context: PictureContext, kit: Kit) -> Self {
        Self {
            art,
            row,
            context,
            kit,
            width: CHOICE_PICTURE_WIDTH,
            height: CHOICE_PICTURE_HEIGHT,
        }
    }

    fn sized(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }
}

impl RenderOnce for SettingsChoiceArt {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let SettingsChoiceArt {
            art,
            row,
            context,
            kit,
            width,
            height,
        } = self;
        let scale = window.scale_factor();
        let tones = choice_tones(row, kit);
        if let ChoiceArt::Mark(mark) = art {
            return div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .w(px(width))
                .h(px(height))
                .child(crate::icon::mark(mark, WELL_MARK_SIZE, tones.line));
        }
        let art_image = |tone: Tone| {
            let image = match &art {
                ChoiceArt::Picture(spec) => {
                    pictures::fitted_image(spec, tones.line, tone, width, height, scale, &context)
                }
                ChoiceArt::Stack { front, back } => pictures::stacked_image(
                    front, back, tones.line, tone, width, height, scale, &context,
                ),
                ChoiceArt::Mark(_) => None,
            };
            image.map(|image| img(image).w(px(width)).h(px(height)))
        };
        let art_box = div().relative().flex_none().w(px(width)).h(px(height));
        if tones.wakes_on_hover {
            art_box
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(0.0)
                        .group_hover(SETTINGS_ROW_GROUP, |style| style.opacity(1.0))
                        .children(art_image(Tone::Awake)),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .group_hover(SETTINGS_ROW_GROUP, |style| style.opacity(0.0))
                        .children(art_image(Tone::Rest)),
                )
        } else {
            art_box.children(art_image(Tone::Awake))
        }
    }
}

/// The picture well of a row that opens something: one size on every row,
/// holding the picture of the picked value or the mark of the row's plugin.
#[derive(IntoElement)]
pub struct SettingsWell {
    art: ChoiceArt,
    row: RowGround,
    kit: Kit,
}

impl SettingsWell {
    pub fn new(art: ChoiceArt, row: RowGround, kit: Kit) -> Self {
        Self { art, row, kit }
    }
}

impl RenderOnce for SettingsWell {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let context = PictureContext::for_accent(
            qol_theme::runtime_theme().mode,
            qol_theme::runtime_accent_key(),
        );
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .w(px(WELL_WIDTH))
            .h(px(WELL_HEIGHT))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .bg(rgba(self.row.rest(self.kit).well.packed()))
            .child(
                SettingsChoiceArt::new(self.art, self.row, context, self.kit)
                    .sized(WELL_PICTURE_WIDTH, WELL_PICTURE_HEIGHT),
            )
    }
}

/// The open part. Only `SettingsRow::opens` draws it, so a row that opens
/// something always ends in it and nothing else draws one.
#[derive(IntoElement)]
pub(super) struct SettingsChevron {
    row: RowGround,
    kit: Kit,
}

impl SettingsChevron {
    pub(super) fn new(row: RowGround, kit: Kit) -> Self {
        Self { row, kit }
    }
}

impl RenderOnce for SettingsChevron {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tones = choice_tones(self.row, self.kit);
        let scale = window.scale_factor();
        let chevron = pictures::chevron(
            tones.chevron,
            (CHOICE_CHEVRON_WIDTH * scale).round() as u32,
            (CHOICE_CHEVRON_HEIGHT * scale).round() as u32,
        )
        .map(|image| {
            img(image)
                .w(px(CHOICE_CHEVRON_WIDTH))
                .h(px(CHOICE_CHEVRON_HEIGHT))
        });
        div()
            .relative()
            .left_0()
            .flex_none()
            .w(px(CHOICE_CHEVRON_WIDTH))
            .h(px(CHOICE_CHEVRON_HEIGHT))
            .opacity(tones.chevron_opacity)
            .group_hover(SETTINGS_ROW_GROUP, |style| {
                style.opacity(1.0).left(px(CHOICE_CHEVRON_HOVER_STEP))
            })
            .children(chevron)
    }
}

#[cfg(test)]
mod tests {
    use super::{choice_tones, ChoiceTones, RowGround, CHOICE_CHEVRON_REST_OPACITY};
    use crate::kit::Kit;
    use crate::theme::{ThemeMode, DARK_SYSTEM};

    fn kit() -> Kit {
        Kit::new(ThemeMode::Dark, DARK_SYSTEM.with_accent(0x8a93f7))
    }

    #[test]
    fn choice_tones_follow_the_row_ground() {
        let kit = kit();
        assert_eq!(
            choice_tones(RowGround::Pane, kit),
            ChoiceTones {
                line: 0xb3b1ac,
                chevron: 0x8b8880,
                chevron_opacity: CHOICE_CHEVRON_REST_OPACITY,
                wakes_on_hover: true,
            }
        );
        assert_eq!(
            choice_tones(RowGround::Band, kit),
            ChoiceTones {
                line: 0xf3f2f0,
                chevron: 0xb8b9c8,
                chevron_opacity: 1.0,
                wakes_on_hover: false,
            }
        );
    }
}
