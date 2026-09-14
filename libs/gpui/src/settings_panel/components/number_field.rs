use gpui::prelude::*;
use gpui::{div, px, rgb, rgba};

use crate::theme::SettingsPanelPalette;

use super::{ground_bg, ground_text, RowGround};

pub(in crate::settings_panel) fn number_field(
    display: String,
    unit: Option<&'static str>,
    fraction: Option<f32>,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let ground = row.rest(palette);
    let hover = row.hover(palette);
    let (text, text_hover) = match row {
        RowGround::Pane => (ground.soft, None),
        RowGround::Band => (ground.ink, hover.map(|hover| hover.ink)),
    };
    let mut cell = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET));
    if let Some(fraction) = fraction {
        cell = cell.child(
            ground_bg(
                div()
                    .relative()
                    .w(px(72.))
                    .h(px(4.))
                    .rounded_full()
                    .overflow_hidden(),
                rgba(ground.well.packed()),
                hover.map(|hover| rgba(hover.well.packed())),
            )
            .child(ground_bg(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(px(fraction * 72.0))
                    .rounded_full(),
                rgb(ground.mark),
                hover.map(|hover| rgb(hover.mark)),
            )),
        );
    }
    let chip = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET))
        .h(px(qol_theme::HEIGHT_INLINE))
        .px(px(qol_theme::SPACE_INSET))
        .rounded(px(qol_theme::RADIUS_CONTROL))
        .border(px(1.0))
        .border_color(rgba(ground.edge.packed()))
        .child(
            ground_text(
                div().text_size(px(qol_theme::TEXT_BODY)),
                rgb(text),
                text_hover.map(rgb),
            )
            .child(display),
        )
        .children(unit.map(|unit| {
            ground_text(
                div().text_size(px(qol_theme::TEXT_CAPTION)),
                rgb(ground.faint),
                hover.map(|hover| rgb(hover.faint)),
            )
            .child(unit)
        }));
    cell.child(ground_bg(
        chip,
        rgba(ground.well.packed()),
        hover.map(|hover| rgba(hover.well.packed())),
    ))
}
