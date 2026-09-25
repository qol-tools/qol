use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, AnyElement, App, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use crate::kit::kit;
use crate::text_edit::{CaretStyle, TextField, TextFieldElement};
use crate::theme::SettingsPanelPalette;

use super::{ground_bg, ground_text, RowGround, FIELD_MAX_WIDTH, TEXT_FIELD_MIN_WIDTH};

fn editable_element(
    field: &TextField,
    visible: usize,
    advance: f32,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> AnyElement {
    let ground = row.rest(palette);
    TextFieldElement::new(field, visible, advance)
        .selection(
            rgb(palette.row_bg_selected).into(),
            Some(rgb(palette.section_text).into()),
        )
        .caret(CaretStyle {
            color: rgb(ground.ink).into(),
            width: 2.0,
            height: 16.0,
            top: 1.0,
            radius: 1.0,
        })
        .render()
        .into_any_element()
}

#[derive(IntoElement)]
pub struct SettingsTextField {
    text: SharedString,
    empty: bool,
    focused: bool,
    row: RowGround,
    palette: SettingsPanelPalette,
    editable: Option<AnyElement>,
    live: Option<TextField>,
    placeholder: Option<SharedString>,
    width: Option<f32>,
}

impl SettingsTextField {
    pub fn new(
        text: impl Into<SharedString>,
        empty: bool,
        focused: bool,
        row: RowGround,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: text.into(),
            empty,
            focused,
            row,
            palette,
            editable: None,
            live: None,
            placeholder: None,
            width: None,
        }
    }

    pub fn editable(
        field: &TextField,
        visible: usize,
        advance: f32,
        focused: bool,
        row: RowGround,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: field.text().to_owned().into(),
            empty: field.is_empty(),
            focused,
            row,
            palette,
            editable: Some(editable_element(field, visible, advance, row, palette)),
            live: None,
            placeholder: None,
            width: None,
        }
    }

    pub fn live(field: TextField, row: RowGround, palette: SettingsPanelPalette) -> Self {
        Self {
            text: field.text().to_owned().into(),
            empty: field.is_empty(),
            focused: true,
            row,
            palette,
            editable: None,
            live: Some(field),
            placeholder: None,
            width: None,
        }
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

impl RenderOnce for SettingsTextField {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let SettingsTextField {
            text,
            empty,
            focused,
            row,
            palette,
            editable,
            live,
            placeholder,
            width,
        } = self;
        let (editable, width) = match live {
            Some(field) => {
                let mono = gpui::font(qol_theme::font_mono());
                let advance =
                    crate::text::shaped_width(window, "0", mono.clone(), qol_theme::TEXT_CAPTION);
                let content =
                    crate::text::shaped_width(window, field.text(), mono, qol_theme::TEXT_CAPTION);
                let width = (content + 2.0 * qol_theme::SPACE_CELL + 2.0)
                    .clamp(TEXT_FIELD_MIN_WIDTH, FIELD_MAX_WIDTH);
                let visible = crate::text_edit::visible_char_count(
                    width - 2.0 * qol_theme::SPACE_CELL - 4.0,
                    advance,
                );
                let element = editable_element(&field, visible, advance, row, palette);
                (Some(element), Some(width))
            }
            None => (editable, width),
        };
        let ground = row.rest(palette);
        let hover = row.hover(palette);
        let band = row == RowGround::Band;
        let placeholder = placeholder.filter(|_| empty && !focused);
        let truncate_plain = editable.is_none() && placeholder.is_none();
        let content = match editable {
            Some(element) => div().w_full().truncate().child(element).into_any_element(),
            None => {
                placeholder.map_or_else(|| text.into_any_element(), IntoElement::into_any_element)
            }
        };
        let (rest_text, hover_text) = if empty && !focused {
            (ground.faint, hover.map(|hover| hover.faint))
        } else {
            match row {
                RowGround::Pane => (ground.soft, None),
                RowGround::Band => (ground.ink, hover.map(|hover| hover.ink)),
            }
        };
        let field = div()
            .flex()
            .items_center()
            .when_some(width, |field, width| field.w(px(width)))
            .min_w(px(TEXT_FIELD_MIN_WIDTH))
            .max_w(px(FIELD_MAX_WIDTH))
            .h(px(qol_theme::HEIGHT_CONTROL))
            .px(px(qol_theme::SPACE_CELL))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .when(band, |field| {
                field
                    .border(px(qol_theme::LINE))
                    .border_color(rgba(ground.edge.packed()))
            })
            .when(focused, |field| field.shadow(kit().focus_ring(ground)))
            .text(TextStyle::Code)
            .overflow_hidden();
        let field = ground_bg(
            field,
            rgba(ground.well.packed()),
            hover.map(|hover| rgba(hover.well.packed())),
        );
        field.child(
            ground_text(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(truncate_plain, |text| text.truncate()),
                rgb(rest_text),
                hover_text.map(rgb),
            )
            .child(content),
        )
    }
}
