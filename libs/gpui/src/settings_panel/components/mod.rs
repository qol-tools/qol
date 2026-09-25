use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, ElementId, Rgba, SharedString};
use qol_theme::TextStyle;

use crate::busy::Busy;
use crate::kit::kit;
use crate::theme::{Ground, SettingsPanelPalette};

mod choice_value;
mod display_layout_tile;
mod feedback;
mod group_header;
mod hint_bar;
mod key_combination;
mod modifier_chip;
mod number_field;
mod qr_code;
mod row;
mod text_field;
mod tile;
mod toggle;

pub use choice_value::{ChoiceArt, SettingsChoiceValue};
pub use display_layout_tile::{
    display_layout_ghost, display_layout_stage, display_layout_tile, display_layout_tile_style,
    DisplayLayoutTile, DisplayLayoutTileStyle,
};
pub use feedback::SettingsFeedback;
pub use group_header::SettingsGroupHeader;
pub use hint_bar::{hint_tone_color, HintTone, SettingsHint, SettingsHintBar};
pub use key_combination::SettingsKeyCombination;
pub use modifier_chip::SettingsModifierChip;
pub(super) use number_field::{number_field, SliderStyle};
pub(super) use qr_code::qr_code_display;
pub use row::SettingsRow;
pub use text_field::SettingsTextField;
pub use tile::{
    choose_hints, choose_step, settings_tile_rows, tile_arts, tile_grid_gap, tile_layout,
    SettingsTile, TileArt, TileLayout, TILE_HEIGHT,
};
pub use toggle::SettingsToggle;

pub const DIMMED_OPACITY: f32 = qol_theme::OPACITY_DISABLED;
const CHOICE_WORD_MAX_WIDTH: f32 = 180.0;
const CHOICE_PICTURE_WIDTH: f32 = 56.0;
const CHOICE_PICTURE_HEIGHT: f32 = 35.0;
const CHOICE_CHEVRON_WIDTH: f32 = 8.0;
const CHOICE_CHEVRON_HEIGHT: f32 = 14.0;
const CHOICE_CHEVRON_REST_OPACITY: f32 = qol_theme::OPACITY_REST;
const TEXT_FIELD_MIN_WIDTH: f32 = 220.0;
const FIELD_MAX_WIDTH: f32 = 320.0;
const TILE_SPINNER_SIZE: f32 = 44.0;

pub const SETTINGS_ROW_GROUP: &str = "settings-row";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowGround {
    Pane,
    Band,
}

impl RowGround {
    pub fn of(selected: bool, focused: bool) -> Self {
        if selected && focused {
            Self::Band
        } else {
            Self::Pane
        }
    }

    pub fn rest(self, palette: SettingsPanelPalette) -> Ground {
        match self {
            Self::Pane => palette.grounds.pane,
            Self::Band => palette.grounds.band,
        }
    }

    pub fn hover(self, palette: SettingsPanelPalette) -> Option<Ground> {
        match self {
            Self::Pane => None,
            Self::Band => Some(palette.grounds.band_hover),
        }
    }
}

fn ground_text<E: InteractiveElement + Styled>(element: E, rest: Rgba, hover: Option<Rgba>) -> E {
    let element = element.text_color(rest);
    match hover {
        Some(color) => {
            element.group_hover(SETTINGS_ROW_GROUP, move |style| style.text_color(color))
        }
        None => element,
    }
}

fn ground_bg<E: InteractiveElement + Styled>(element: E, rest: Rgba, hover: Option<Rgba>) -> E {
    let element = element.bg(rest);
    match hover {
        Some(color) => element.group_hover(SETTINGS_ROW_GROUP, move |style| style.bg(color)),
        None => element,
    }
}

fn ground_border<E: InteractiveElement + Styled>(element: E, rest: Rgba, hover: Option<Rgba>) -> E {
    let element = element.border_color(rest);
    match hover {
        Some(color) => {
            element.group_hover(SETTINGS_ROW_GROUP, move |style| style.border_color(color))
        }
        None => element,
    }
}

fn masthead_rule() -> gpui::Div {
    div()
        .flex_none()
        .h(px(1.0))
        .bg(rgba(kit().washes.hairline.packed()))
}

pub fn paint_settings_selection<E: Styled>(row: E, palette: SettingsPanelPalette) -> E {
    row.relative()
        .w_auto()
        .mx(px(-qol_theme::SPACE_PAD))
        .px(px(qol_theme::SPACE_PAD + qol_theme::SPACE_INSET))
        .rounded_none()
        .bg(rgb(palette.grounds.band.bg))
}

pub fn paint_rail_selection<E: Styled>(row: E, palette: SettingsPanelPalette, focused: bool) -> E {
    row.bg(rgb(if focused {
        palette.fill_current
    } else {
        palette.fill_current_quiet
    }))
}

fn paint_settings_attention<E: Styled + ParentElement>(row: E, palette: SettingsPanelPalette) -> E {
    row.bg(rgb(palette.grounds.attention.bg))
}

fn attention_dot(palette: SettingsPanelPalette) -> gpui::Div {
    let shared = kit();
    shared.status_dot(
        palette.grounds.attention.mark,
        shared.washes.halo_attention.packed(),
    )
}

pub fn one_line<E: Styled>(element: E) -> E {
    element.line_clamp(1).text_ellipsis()
}

pub fn settings_label(text: impl Into<SharedString>, palette: SettingsPanelPalette) -> gpui::Div {
    div()
        .text(TextStyle::Name)
        .text_color(rgb(palette.section_text))
        .child(text.into())
}

pub fn settings_description(
    text: impl Into<SharedString>,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let ground = row.rest(palette);
    let (rest, hover) = match row {
        RowGround::Pane => (ground.faint, None),
        RowGround::Band => (ground.soft, row.hover(palette).map(|hover| hover.soft)),
    };
    ground_text(
        div().text(TextStyle::Detail).child(text.into()),
        rgb(rest),
        hover.map(rgb),
    )
}

pub fn settings_value_group() -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(px(qol_theme::SPACE_INSET))
}

pub fn settings_mono_label(
    text: impl Into<SharedString>,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let ground = row.rest(palette);
    let (rest, hover) = match row {
        RowGround::Pane => (ground.soft, None),
        RowGround::Band => (ground.ink, row.hover(palette).map(|hover| hover.ink)),
    };
    ground_text(
        div()
            .flex_1()
            .min_w_0()
            .text(TextStyle::Code)
            .child(text.into()),
        rgb(rest),
        hover.map(rgb),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsValueTone {
    Normal,
    Muted,
    Attention,
    Danger,
    Success,
}

pub fn settings_value_text(
    text: impl Into<SharedString>,
    tone: SettingsValueTone,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let value = kit().value(text);
    let ground = row.rest(palette);
    let hover = row.hover(palette);
    match tone {
        SettingsValueTone::Normal => {
            let (rest, hover) = match row {
                RowGround::Pane => (ground.soft, None),
                RowGround::Band => (ground.ink, hover.map(|hover| hover.ink)),
            };
            ground_text(value, rgb(rest), hover.map(rgb))
        }
        SettingsValueTone::Muted => {
            let (rest, hover) = match row {
                RowGround::Pane => (ground.faint, None),
                RowGround::Band => (ground.soft, hover.map(|hover| hover.soft)),
            };
            ground_text(value, rgb(rest), hover.map(rgb))
        }
        SettingsValueTone::Attention => value.text_color(rgb(palette.status_warning_ink)),
        SettingsValueTone::Danger => value.text_color(rgb(palette.status_danger)),
        SettingsValueTone::Success => value.text_color(rgb(palette.status_success)),
    }
}

pub fn settings_action_affordance(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: Option<&str>,
    busy: bool,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let ground = row.rest(palette);
    let hover = row.hover(palette);
    let band = row == RowGround::Band;
    let (background, text) = if band && variant != Some("danger") {
        (rgba(ground.well.packed()), ground.ink)
    } else {
        match variant {
            Some("ghost") => (rgb(palette.surface_raised), palette.label_text),
            Some("danger") => (
                rgba(qol_theme::translucent(
                    palette.state_off,
                    qol_theme::Alpha::Halo,
                )),
                palette.state_off,
            ),
            Some("primary") | None | Some(_) => {
                (rgb(palette.row_bg_selected), palette.section_text)
            }
        }
    };
    let mut control = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_TIGHT));
    if busy {
        control = control.child(settings_action_spinner(id, palette).size(px(12.)));
    }
    let control = control
        .px(px(qol_theme::SPACE_INSET))
        .py(px(qol_theme::SPACE_TIGHT))
        .rounded(px(qol_theme::RADIUS_CONTROL))
        .when(variant == Some("ghost") && !band, |control| {
            control.shadow(crate::kit::raised_shadow(palette.section_text))
        })
        .text(TextStyle::ListName);
    let control = if variant == Some("danger") {
        control.bg(background)
    } else {
        ground_bg(
            control,
            background,
            hover.map(|hover| rgba(hover.well.packed())),
        )
    };
    let element = if variant == Some("danger") {
        div().text_color(rgb(text))
    } else {
        ground_text(div(), rgb(text), hover.map(|hover| rgb(hover.ink)))
    };
    control.child(element.child(label.into()))
}

const CRUMB_MAX_WIDTH: f32 = 200.0;

pub fn settings_crumb_trail(trail: Vec<String>, palette: SettingsPanelPalette) -> gpui::Div {
    let last = trail.len().saturating_sub(1);
    let separator = rgba(qol_theme::translucent(
        palette.status_muted,
        qol_theme::Alpha::Veil,
    ));
    let mut crumbs = Vec::with_capacity(trail.len() * 2);
    for (index, label) in trail.into_iter().enumerate() {
        if index > 0 {
            crumbs.push(
                div()
                    .flex_none()
                    .px(px(qol_theme::SPACE_TIGHT))
                    .text_color(separator)
                    .child("/"),
            );
        }
        let crumb = if index == last {
            div().text_color(rgb(palette.section_text))
        } else {
            div()
                .max_w(px(CRUMB_MAX_WIDTH))
                .text_color(rgb(palette.status_muted))
        };
        crumbs.push(crumb.child(label.to_lowercase()));
    }
    div()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .text(TextStyle::Heading)
        .children(crumbs)
}

pub fn rail_caption_height() -> f32 {
    qol_theme::HEIGHT_BAND
}

pub fn rail_caption(
    label: impl Into<SharedString>,
    detail: Option<SharedString>,
    focused: bool,
) -> gpui::Div {
    let kit = kit();
    let label: SharedString = label.into();
    let block = div()
        .flex_none()
        .relative()
        .flex()
        .flex_col()
        .justify_center()
        .w_full()
        .h(px(rail_caption_height()))
        .px(px(qol_theme::SPACE_CELL))
        .child(kit.heading_title(
            TextStyle::Masthead,
            SharedString::from(label.to_lowercase()),
            detail.map(|detail| SharedString::from(detail.to_lowercase())),
            focused,
        ));
    block.child(
        masthead_rule()
            .absolute()
            .bottom_0()
            .left(px(qol_theme::SPACE_CELL))
            .right(px(qol_theme::SPACE_CELL)),
    )
}

pub fn settings_page() -> gpui::Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .px(px(qol_theme::SPACE_PAD))
        .pb(px(qol_theme::SPACE_PAD))
        .gap(px(qol_theme::SPACE_TIGHT))
}

pub fn settings_label_group(
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(qol_theme::SPACE_STACK))
        .child(settings_label(label, palette))
        .children(description.map(|text| settings_description(text, row, palette)))
}

pub fn settings_message(
    text: impl Into<SharedString>,
    danger: bool,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let kit = kit();
    if danger {
        settings_message_frame(palette.status_danger).child(kit.notice(
            crate::kit::NoticeTone::Invalid,
            text,
            None,
        ))
    } else {
        kit.empty(text, None)
    }
}

fn settings_message_frame(color: u32) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text(TextStyle::Value)
        .text_color(rgb(color))
}

/// Busy recipe for a query-backed value that has not answered yet.
pub fn settings_query_spinner(
    id: impl Into<ElementId>,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> Busy {
    let color = match row {
        RowGround::Pane => palette.grounds.pane.faint,
        RowGround::Band => palette.grounds.band.soft,
    };
    Busy::ring(id, rgb(color))
}

pub fn settings_tile_spinner(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Busy {
    Busy::ring(id, rgb(palette.grounds.pane.faint)).size(px(TILE_SPINNER_SIZE))
}

/// Busy recipe for a pending action inside a settings surface.
pub fn settings_action_spinner(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Busy {
    Busy::ring(id, rgb(palette.state_on))
}

/// Busy recipe sharing the settings_message frame for in-progress work.
pub fn settings_busy_message(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    settings_message_frame(palette.status_muted).child(Busy::new(
        id,
        text,
        rgb(palette.status_muted),
    ))
}

#[cfg(test)]
mod tests {
    use super::RowGround;

    #[test]
    fn row_ground_is_band_only_with_selection_and_body_focus() {
        assert_eq!(RowGround::of(false, false), RowGround::Pane);
        assert_eq!(RowGround::of(false, true), RowGround::Pane);
        assert_eq!(RowGround::of(true, false), RowGround::Pane);
        assert_eq!(RowGround::of(true, true), RowGround::Band);
    }
}
