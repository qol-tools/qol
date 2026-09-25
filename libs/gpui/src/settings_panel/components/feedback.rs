use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use crate::kit::kit;

#[derive(IntoElement)]
pub struct SettingsFeedback {
    message: SharedString,
    tone: u32,
    danger: bool,
}

impl SettingsFeedback {
    pub fn new(message: impl Into<SharedString>, tone: u32, danger: bool) -> Self {
        Self {
            message: message.into(),
            tone,
            danger,
        }
    }
}

impl RenderOnce for SettingsFeedback {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        let (ground, halo) = if self.danger {
            (shared.grounds.invalid, shared.washes.halo_invalid)
        } else {
            (shared.grounds.attention, shared.washes.halo_attention)
        };
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_CELL))
            .px(px(qol_theme::SPACE_GUTTER))
            .py(px(qol_theme::SPACE_INSET))
            .border_t(px(1.0))
            .border_color(rgba(shared.washes.hairline.packed()))
            .bg(rgb(ground.bg))
            .child(shared.status_dot(self.tone, halo.packed()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text(TextStyle::Detail)
                    .text_color(rgb(ground.ink))
                    .child(self.message),
            )
    }
}
