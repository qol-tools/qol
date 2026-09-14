use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, RenderOnce, Window};

use crate::theme::SettingsPanelPalette;

use super::{ground_bg, RowGround};

const TOGGLE_TRACK_WIDTH: f32 = 40.0;
const TOGGLE_TRACK_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - 4.0;

#[derive(IntoElement)]
pub struct SettingsToggle {
    active: bool,
    row: RowGround,
    palette: SettingsPanelPalette,
}

impl SettingsToggle {
    pub fn new(active: bool, row: RowGround, palette: SettingsPanelPalette) -> Self {
        Self {
            active,
            row,
            palette,
        }
    }
}

impl RenderOnce for SettingsToggle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ground = self.row.rest(self.palette);
        let hover = self.row.hover(self.palette);
        let (track, track_hover) = if self.active {
            (rgb(ground.mark), hover.map(|hover| rgb(hover.mark)))
        } else {
            (
                rgba(ground.well.packed()),
                hover.map(|hover| rgba(hover.well.packed())),
            )
        };
        let (knob, knob_hover) = if self.active {
            (rgb(ground.on_mark), hover.map(|hover| rgb(hover.on_mark)))
        } else {
            (rgb(ground.soft), hover.map(|hover| rgb(hover.soft)))
        };
        div().flex().flex_row().items_center().child(
            ground_bg(
                div()
                    .flex()
                    .items_center()
                    .when(self.active, |track| track.justify_end())
                    .when(!self.active, |track| track.justify_start())
                    .w(px(TOGGLE_TRACK_WIDTH))
                    .h(px(TOGGLE_TRACK_HEIGHT))
                    .p(px(qol_theme::SPACE_STACK))
                    .rounded_full(),
                track,
                track_hover,
            )
            .child(ground_bg(
                div()
                    .w(px(TOGGLE_TRACK_HEIGHT - 2.0 * qol_theme::SPACE_STACK))
                    .h(px(TOGGLE_TRACK_HEIGHT - 2.0 * qol_theme::SPACE_STACK))
                    .rounded_full(),
                knob,
                knob_hover,
            )),
        )
    }
}
