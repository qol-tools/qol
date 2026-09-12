use gpui::prelude::*;
use gpui::{div, px, rgb, AnyElement, App, RenderOnce, SharedString, Window};

use crate::kit::kit;
use crate::text_edit::{CaretStyle, TextField, TextFieldElement};
use crate::theme::SettingsPanelPalette;

use super::{FIELD_MAX_WIDTH, FIELD_MIN_WIDTH};

#[derive(IntoElement)]
pub struct SettingsTextField {
    text: SharedString,
    empty: bool,
    focused: bool,
    palette: SettingsPanelPalette,
    editable: Option<AnyElement>,
    placeholder: Option<SharedString>,
    width: Option<f32>,
}

impl SettingsTextField {
    pub fn new(
        text: impl Into<SharedString>,
        empty: bool,
        focused: bool,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: text.into(),
            empty,
            focused,
            palette,
            editable: None,
            placeholder: None,
            width: None,
        }
    }

    pub fn editable(
        field: &TextField,
        visible: usize,
        advance: f32,
        focused: bool,
        palette: SettingsPanelPalette,
    ) -> Self {
        let editable = TextFieldElement::new(field, visible, advance)
            .selection(
                rgb(palette.row_bg_selected).into(),
                Some(rgb(palette.section_text).into()),
            )
            .caret(CaretStyle {
                color: rgb(palette.row_border_selected).into(),
                width: 2.0,
                height: 16.0,
                top: 1.0,
                radius: 1.0,
            })
            .render()
            .into_any_element();
        Self {
            text: field.text().to_owned().into(),
            empty: field.is_empty(),
            focused,
            palette,
            editable: Some(editable),
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
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        let placeholder = self.placeholder.filter(|_| self.empty && !self.focused);
        let editable = self.editable;
        let truncate_plain = editable.is_none() && placeholder.is_none();
        let content = match editable {
            Some(element) => div().w_full().truncate().child(element).into_any_element(),
            None => placeholder.map_or_else(
                || self.text.into_any_element(),
                IntoElement::into_any_element,
            ),
        };
        div()
            .flex()
            .items_center()
            .when_some(self.width, |field, width| field.w(px(width)))
            .min_w(px(FIELD_MIN_WIDTH))
            .max_w(px(FIELD_MAX_WIDTH))
            .h(px(qol_theme::HEIGHT_CONTROL))
            .px(px(qol_theme::SPACE_CELL))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .border(px(1.0))
            .border_color(rgb(if self.focused {
                self.palette.row_border_selected
            } else {
                self.palette.panel_border
            }))
            .bg(rgb(self.palette.dropdown_bg))
            .when(self.focused, |field| field.shadow(shared.focus_ring()))
            .font_family(SharedString::from(qol_theme::font_mono()))
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(if self.empty && !self.focused {
                self.palette.status_muted
            } else {
                self.palette.section_text
            }))
            .when(truncate_plain, |field| field.truncate())
            .overflow_hidden()
            .child(content)
    }
}
