use gpui::prelude::*;
use gpui::{div, px, rgb};

use crate::theme::SettingsPanelPalette;

pub(in crate::settings_panel) fn number_field(
    display: String,
    unit: Option<&'static str>,
    fraction: Option<f32>,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let mut cell = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET));
    if let Some(fraction) = fraction {
        cell = cell.child(
            div()
                .relative()
                .w(px(72.))
                .h(px(4.))
                .rounded_full()
                .overflow_hidden()
                .bg(rgb(palette.panel_border))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(px(fraction * 72.0))
                        .rounded_full()
                        .bg(rgb(palette.row_border_selected)),
                ),
        );
    }
    cell.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_TIGHT))
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .bg(rgb(palette.dropdown_bg))
            .text_size(px(qol_theme::TEXT_BODY))
            .text_color(rgb(palette.label_text))
            .child(display)
            .children(unit.map(|unit| {
                div()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(palette.status_muted))
                    .child(unit)
            })),
    )
}
