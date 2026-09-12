use gpui::prelude::*;
use gpui::{div, px, rgb, App, RenderOnce, Window};

use crate::theme::SettingsPanelPalette;

const TOGGLE_TRACK_WIDTH: f32 = 40.0;
const TOGGLE_TRACK_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - 4.0;

#[derive(IntoElement)]
pub struct SettingsToggle {
    active: bool,
    palette: SettingsPanelPalette,
}

impl SettingsToggle {
    pub fn new(active: bool, palette: SettingsPanelPalette) -> Self {
        Self { active, palette }
    }
}

impl RenderOnce for SettingsToggle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().flex().flex_row().items_center().child(
            div()
                .flex()
                .items_center()
                .when(self.active, |track| track.justify_end())
                .when(!self.active, |track| track.justify_start())
                .w(px(TOGGLE_TRACK_WIDTH))
                .h(px(TOGGLE_TRACK_HEIGHT))
                .p(px(qol_theme::SPACE_STACK))
                .rounded_full()
                .bg(rgb(if self.active {
                    self.palette.row_border_selected
                } else {
                    self.palette.dropdown_bg
                }))
                .child(
                    div()
                        .w(px(TOGGLE_TRACK_HEIGHT - 2.0 * qol_theme::SPACE_STACK))
                        .h(px(TOGGLE_TRACK_HEIGHT - 2.0 * qol_theme::SPACE_STACK))
                        .rounded_full()
                        .bg(rgb(if self.active {
                            self.palette.window_bg
                        } else {
                            self.palette.label_text
                        })),
                ),
        )
    }
}
