use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, RenderOnce, SharedString, Window};

use super::{ground_bg, ground_text, RowGround, FIELD_MIN_WIDTH, VALUE_MAX_WIDTH};
use crate::theme::SettingsPanelPalette;

#[derive(IntoElement)]
pub struct SettingsSelectValue {
    text: SharedString,
    row: RowGround,
    palette: SettingsPanelPalette,
}

impl SettingsSelectValue {
    pub fn new(
        text: impl Into<SharedString>,
        row: RowGround,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: text.into(),
            row,
            palette,
        }
    }
}

impl RenderOnce for SettingsSelectValue {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ground = self.row.rest(self.palette);
        let hover = self.row.hover(self.palette);
        let (text, hover_text) = match self.row {
            RowGround::Pane => (ground.soft, None),
            RowGround::Band => (ground.ink, hover.map(|hover| hover.ink)),
        };
        let chip = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .h(px(qol_theme::HEIGHT_INLINE))
            .px(px(qol_theme::SPACE_INSET))
            .min_w(px(FIELD_MIN_WIDTH))
            .max_w(px(VALUE_MAX_WIDTH))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .border(px(1.0))
            .border_color(rgba(ground.edge.packed()))
            .text_size(px(qol_theme::TEXT_BODY));
        let chip = ground_bg(
            chip,
            rgba(ground.well.packed()),
            hover.map(|hover| rgba(hover.well.packed())),
        );
        chip.child(
            ground_text(
                div().flex_1().min_w_0().truncate(),
                rgb(text),
                hover_text.map(rgb),
            )
            .child(self.text),
        )
        .child(
            ground_text(
                div().flex_none().text_size(px(qol_theme::TEXT_CAPTION)),
                rgb(ground.faint),
                hover.map(|hover| rgb(hover.faint)),
            )
            .child("▾"),
        )
    }
}
