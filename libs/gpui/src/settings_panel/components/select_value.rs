use gpui::prelude::*;
use gpui::{div, px, rgb, App, RenderOnce, SharedString, Window};

use super::{FIELD_MIN_WIDTH, VALUE_MAX_WIDTH};
use crate::theme::SettingsPanelPalette;

#[derive(IntoElement)]
pub struct SettingsSelectValue {
    text: SharedString,
    accent: Option<u32>,
    palette: SettingsPanelPalette,
}

impl SettingsSelectValue {
    pub fn new(text: impl Into<SharedString>, palette: SettingsPanelPalette) -> Self {
        Self {
            text: text.into(),
            accent: None,
            palette,
        }
    }

    pub fn accent(mut self, accent: Option<u32>) -> Self {
        self.accent = accent;
        self
    }
}

impl RenderOnce for SettingsSelectValue {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .min_w(px(FIELD_MIN_WIDTH))
            .max_w(px(VALUE_MAX_WIDTH))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .bg(rgb(self.palette.dropdown_bg))
            .text_size(px(qol_theme::TEXT_BODY))
            .text_color(rgb(self.palette.label_text))
            .children(
                self.accent
                    .map(|accent| div().flex_none().w_2().h_2().rounded_full().bg(rgb(accent))),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.text))
            .child(
                div()
                    .flex_none()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.status_muted))
                    .child("▾"),
            )
    }
}
