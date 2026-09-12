use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, RenderOnce, SharedString, Window};

use crate::kit::{alpha, kit};

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
        div()
            .flex_none()
            .flex()
            .flex_row()
            .border_t(px(1.0))
            .border_color(rgba(shared.washes.hairline.packed()))
            .bg(rgba(if self.danger {
                shared.washes.wash_invalid.packed()
            } else {
                alpha(self.tone, 0x16)
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(qol_theme::SPACE_MARK))
                    .bg(rgb(self.tone)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .px(px(qol_theme::SPACE_GUTTER))
                    .py(px(qol_theme::SPACE_INSET))
                    .text_size(px(qol_theme::TEXT_MICRO))
                    .text_color(rgb(self.tone))
                    .child(self.message),
            )
    }
}
