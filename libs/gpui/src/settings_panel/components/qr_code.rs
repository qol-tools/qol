use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, ElementId, SharedString};
use qol_theme::TextStyle;

use crate::kit::Kit;
use crate::settings_panel::rows::{Row, RowControl};
use crate::settings_panel::{
    PANEL_LIST_DESCRIPTION_HEIGHT, PANEL_LIST_GAP, PANEL_LIST_HEADER_HEIGHT, PANEL_LIST_PADDING_Y,
    PANEL_QR_CODE_HEIGHT, PANEL_QR_URL_HEIGHT,
};

use super::{one_line, paint_settings_selection, settings_query_spinner, RowGround};

pub(in crate::settings_panel) struct QrBlock<'a> {
    pub label: &'a str,
    pub description: Option<&'a str>,
    pub placeholder: Option<&'a str>,
    pub url: Option<&'a str>,
    pub modules: &'a [bool],
    pub error: Option<&'a str>,
}

pub(in crate::settings_panel) fn qr_heights(described: bool) -> (f32, f32) {
    let header = PANEL_LIST_HEADER_HEIGHT
        + if described {
            PANEL_LIST_DESCRIPTION_HEIGHT
        } else {
            0.0
        };
    let body = PANEL_LIST_PADDING_Y
        + header
        + PANEL_QR_CODE_HEIGHT
        + PANEL_QR_URL_HEIGHT
        + 2.0 * PANEL_LIST_GAP;
    (body, header)
}

pub(in crate::settings_panel) fn qr_code_display(
    row: &Row,
    index: usize,
    highlighted: bool,
    loading: bool,
    body_height: f32,
    header_height: f32,
    kit: Kit,
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
    qr_block(
        QrBlock {
            label: &row.label,
            description: row.description.as_deref(),
            placeholder: row.placeholder.as_deref(),
            url: url.as_deref(),
            modules,
            error: error.as_deref(),
        },
        ("settings-qr-spinner", index),
        highlighted,
        loading,
        body_height,
        header_height,
        kit,
    )
}

pub(in crate::settings_panel) fn qr_block(
    block: QrBlock<'_>,
    spinner: impl Into<ElementId>,
    highlighted: bool,
    loading: bool,
    body_height: f32,
    header_height: f32,
    kit: Kit,
) -> gpui::Div {
    let QrBlock {
        label,
        description,
        placeholder,
        url,
        modules,
        error,
    } = block;
    let row_ground = RowGround::of(highlighted, true);
    let ground = row_ground.rest(kit);
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
        container = paint_settings_selection(container, kit);
    }
    container = container.child(
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(header_height))
            .gap(px(qol_theme::SPACE_CELL))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .flex_1()
                    .flex_col()
                    .child(
                        div()
                            .text(TextStyle::Name)
                            .text_color(rgb(ground.ink))
                            .child(SharedString::from(label.to_string())),
                    )
                    .when_some(description, |group, description| {
                        group.child(
                            div()
                                .text(TextStyle::Detail)
                                .text_color(rgb(ground.soft))
                                .child(SharedString::from(description.to_string())),
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
                .bg(rgb(qol_theme::DARK_TRAY_INTERNAL.config_qr_light))
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
                                .bg(rgb(qol_theme::DARK_TRAY_INTERNAL.config_qr_dark)),
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
                frame.child(settings_query_spinner(spinner, row_ground, kit))
            } else {
                frame
                    .text(TextStyle::Value)
                    .text_color(rgb(ground.soft))
                    .child(SharedString::from(
                        placeholder.unwrap_or("unavailable").to_string(),
                    ))
            }
        }
    };
    container = container.child(code_block);
    let status = match (error, url) {
        (Some(message), _) => message.to_string(),
        (None, Some(url)) => url.to_string(),
        (None, None) => String::new(),
    };
    if !status.is_empty() {
        container = container.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(px(PANEL_QR_URL_HEIGHT))
                .text(TextStyle::Code)
                .text_color(if error.is_some() {
                    rgb(kit.palette.danger)
                } else {
                    rgb(ground.soft)
                })
                .child(one_line(div().min_w_0()).child(status)),
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
