use gpui::prelude::*;
use gpui::{div, img, px, rgb, App, RenderOnce, SharedString, Window};

use super::{
    ground_text, RowGround, CHOICE_CHEVRON_HEIGHT, CHOICE_CHEVRON_REST_OPACITY,
    CHOICE_CHEVRON_WIDTH, CHOICE_PICTURE_HEIGHT, CHOICE_PICTURE_WIDTH, CHOICE_WORD_MAX_WIDTH,
    SETTINGS_ROW_GROUP,
};
use crate::pictures::{self, PictureContext, Tone};
use crate::theme::SettingsPanelPalette;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceArt {
    Picture(String),
    Stack { front: String, back: String },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ChoiceTones {
    word: u32,
    word_hover: u32,
    line: u32,
    chevron: u32,
    chevron_opacity: f32,
    wakes_on_hover: bool,
}

fn choice_tones(row: RowGround, palette: SettingsPanelPalette) -> ChoiceTones {
    match row {
        RowGround::Pane => ChoiceTones {
            word: palette.grounds.pane.faint,
            word_hover: palette.grounds.pane.soft,
            line: palette.grounds.pane.soft,
            chevron: palette.grounds.pane.faint,
            chevron_opacity: CHOICE_CHEVRON_REST_OPACITY,
            wakes_on_hover: true,
        },
        RowGround::Band => ChoiceTones {
            word: palette.grounds.band.ink,
            word_hover: palette.grounds.band_hover.ink,
            line: palette.grounds.band.ink,
            chevron: palette.grounds.band.faint,
            chevron_opacity: 1.0,
            wakes_on_hover: false,
        },
    }
}

#[derive(IntoElement)]
pub struct SettingsChoiceValue {
    text: SharedString,
    art: ChoiceArt,
    row: RowGround,
    context: PictureContext,
    palette: SettingsPanelPalette,
}

impl SettingsChoiceValue {
    pub fn new(
        text: impl Into<SharedString>,
        art: ChoiceArt,
        row: RowGround,
        context: PictureContext,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: text.into(),
            art,
            row,
            context,
            palette,
        }
    }
}

impl RenderOnce for SettingsChoiceValue {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let SettingsChoiceValue {
            text,
            art,
            row,
            context,
            palette,
        } = self;
        let scale = window.scale_factor();
        let tones = choice_tones(row, palette);
        let art_image = |tone: Tone| {
            let image = match &art {
                ChoiceArt::Picture(spec) => pictures::fitted_image(
                    spec,
                    tones.line,
                    tone,
                    CHOICE_PICTURE_WIDTH,
                    CHOICE_PICTURE_HEIGHT,
                    scale,
                    &context,
                ),
                ChoiceArt::Stack { front, back } => pictures::stacked_image(
                    front,
                    back,
                    tones.line,
                    tone,
                    CHOICE_PICTURE_WIDTH,
                    CHOICE_PICTURE_HEIGHT,
                    scale,
                    &context,
                ),
            };
            image.map(|image| {
                img(image)
                    .w(px(CHOICE_PICTURE_WIDTH))
                    .h(px(CHOICE_PICTURE_HEIGHT))
            })
        };
        let art_box = div()
            .relative()
            .flex_none()
            .w(px(CHOICE_PICTURE_WIDTH))
            .h(px(CHOICE_PICTURE_HEIGHT));
        let art_box = if tones.wakes_on_hover {
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
        };
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
        let arrow = div()
            .flex_none()
            .w(px(CHOICE_CHEVRON_WIDTH))
            .h(px(CHOICE_CHEVRON_HEIGHT))
            .children(chevron);
        let arrow = if tones.wakes_on_hover {
            arrow
                .opacity(tones.chevron_opacity)
                .group_hover(SETTINGS_ROW_GROUP, |style| style.opacity(1.0))
        } else {
            arrow
        };
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(qol_theme::SPACE_CELL))
            .child(
                ground_text(
                    div()
                        .flex_none()
                        .max_w(px(CHOICE_WORD_MAX_WIDTH))
                        .truncate()
                        .text_size(px(qol_theme::TEXT_BODY)),
                    rgb(tones.word),
                    Some(rgb(tones.word_hover)),
                )
                .child(text),
            )
            .child(art_box)
            .child(arrow)
    }
}

#[cfg(test)]
mod tests {
    use super::{choice_tones, ChoiceTones, RowGround, CHOICE_CHEVRON_REST_OPACITY};
    use crate::theme::{SettingsPanelPalette, ThemeMode, DARK_SYSTEM};

    fn palette() -> SettingsPanelPalette {
        SettingsPanelPalette::from_theme(ThemeMode::Dark, DARK_SYSTEM.with_accent(0x8a93f7))
    }

    #[test]
    fn choice_tones_follow_the_row_ground() {
        let palette = palette();
        assert_eq!(
            choice_tones(RowGround::Pane, palette),
            ChoiceTones {
                word: 0x8b8880,
                word_hover: 0xb3b1ac,
                line: 0xb3b1ac,
                chevron: 0x8b8880,
                chevron_opacity: CHOICE_CHEVRON_REST_OPACITY,
                wakes_on_hover: true,
            }
        );
        assert_eq!(
            choice_tones(RowGround::Band, palette),
            ChoiceTones {
                word: 0xf3f2f0,
                word_hover: 0xf3f2f0,
                line: 0xf3f2f0,
                chevron: 0xb8b9c8,
                chevron_opacity: 1.0,
                wakes_on_hover: false,
            }
        );
    }
}
