use gpui::prelude::*;
use gpui::{div, px, rgb, FontWeight};

use crate::settings_panel::rows::{Row, RowControl};
use crate::settings_panel::{PANEL_QR_CODE_HEIGHT, PANEL_QR_URL_HEIGHT};
use crate::theme::SettingsPanelPalette;

use super::{paint_settings_selection, settings_query_spinner};

pub(in crate::settings_panel) fn qr_code_display(
    row: &Row,
    index: usize,
    highlighted: bool,
    loading: bool,
    body_height: f32,
    header_height: f32,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let RowControl::QrCode {
        url,
        modules,
        error,
        ..
    } = &row.control
    else {
        return div();
    };
    let mut container = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(qol_theme::SPACE_TIGHT))
        .h(px(body_height))
        .overflow_hidden()
        .px(px(qol_theme::SPACE_INSET))
        .py(px(qol_theme::SPACE_TIGHT))
        .rounded(px(qol_theme::RADIUS_CARD));
    if highlighted {
        container = paint_settings_selection(container, palette);
    }
    container = container.child(
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(header_height))
            .gap(px(qol_theme::SPACE_CELL))
            .text_size(px(qol_theme::TEXT_BODY))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .flex_1()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.section_text))
                            .child(row.label.clone()),
                    )
                    .when_some(row.description.clone(), |group, description| {
                        group.child(
                            div()
                                .truncate()
                                .text_size(px(qol_theme::TEXT_CAPTION))
                                .text_color(rgb(palette.label_text))
                                .child(description),
                        )
                    }),
            ),
    );
    let code_block = match url {
        Some(_) => {
            let module_px = qr_module_px(modules);
            let side = qr_side(modules);
            let mut grid = div()
                .flex()
                .flex_col()
                .flex_none()
                .items_center()
                .justify_center()
                .bg(rgb(palette.qr_light))
                .h(px(PANEL_QR_CODE_HEIGHT));
            for y in 0..side {
                let mut line = div().flex().flex_row().flex_none().h(px(module_px));
                for x in 0..side {
                    let dark = modules[y * side + x];
                    if dark {
                        line = line.child(
                            div()
                                .w(px(module_px))
                                .h(px(module_px))
                                .bg(rgb(palette.qr_dark)),
                        );
                    } else {
                        line = line.child(div().w(px(module_px)).h(px(module_px)));
                    }
                }
                grid = grid.child(line);
            }
            grid
        }
        None => {
            let frame = div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(PANEL_QR_CODE_HEIGHT));
            if loading {
                frame.child(settings_query_spinner(
                    ("settings-qr-spinner", index),
                    palette,
                ))
            } else {
                let placeholder = row
                    .placeholder
                    .clone()
                    .unwrap_or_else(|| "unavailable".into());
                frame
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(palette.label_text))
                    .child(placeholder)
            }
        }
    };
    container = container.child(code_block);
    let status = match (error.as_ref(), url.as_ref()) {
        (Some(message), _) => message.clone(),
        (None, Some(url)) => url.clone(),
        (None, None) => String::new(),
    };
    if !status.is_empty() {
        container = container.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(px(PANEL_QR_URL_HEIGHT))
                .text_size(px(qol_theme::TEXT_CAPTION))
                .text_color(if error.is_some() {
                    rgb(palette.state_off)
                } else {
                    rgb(palette.label_text)
                })
                .child(status),
        );
    }
    container
}

fn qr_side(modules: &[bool]) -> usize {
    (modules.len() as f64).sqrt() as usize
}

fn qr_module_px(modules: &[bool]) -> f32 {
    let side = qr_side(modules);
    let module = PANEL_QR_CODE_HEIGHT / side as f32;
    module.clamp(2.0, 4.0)
}
