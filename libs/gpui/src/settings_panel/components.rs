use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId, FontWeight, RenderOnce,
    SharedString, Window,
};

use crate::dropdown::DropdownStyle;
use crate::kit::{alpha, kit};
use crate::spinner::{Busy, Spinner};
use crate::text_edit::{CaretStyle, TextField, TextFieldElement};
use crate::theme::SettingsPanelPalette;

pub const DIMMED_OPACITY: f32 = 0.5;
const TOGGLE_TRACK_WIDTH: f32 = 40.0;
const TOGGLE_TRACK_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - 4.0;
const FIELD_MIN_WIDTH: f32 = 180.0;
const VALUE_MAX_WIDTH: f32 = 280.0;
const FIELD_MAX_WIDTH: f32 = 320.0;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Setting,
    Rule,
    Add,
}

#[derive(IntoElement)]
pub struct SettingsRow {
    id: ElementId,
    palette: SettingsPanelPalette,
    kind: RowKind,
    selected: bool,
    focused: bool,
    dimmed: bool,
    attention: bool,
    children: Vec<AnyElement>,
    on_click: Option<ClickHandler>,
}

impl SettingsRow {
    pub fn setting(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Self {
        Self::new(id, palette, RowKind::Setting)
    }

    pub fn rule(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Self {
        Self::new(id, palette, RowKind::Rule)
    }

    pub fn add(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Self {
        Self::new(id, palette, RowKind::Add)
    }

    fn new(id: impl Into<ElementId>, palette: SettingsPanelPalette, kind: RowKind) -> Self {
        Self {
            id: id.into(),
            palette,
            kind,
            selected: false,
            focused: true,
            dimmed: false,
            attention: false,
            children: Vec::new(),
            on_click: None,
        }
    }

    pub fn selected(mut self, selected: bool, focused: bool) -> Self {
        self.selected = selected;
        self.focused = focused;
        self
    }

    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    pub fn attention(mut self, attention: bool) -> Self {
        self.attention = attention;
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = AnyElement>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        let height = match self.kind {
            RowKind::Setting => qol_theme::HEIGHT_SETTING_ROW,
            RowKind::Rule | RowKind::Add => qol_theme::HEIGHT_RULE_ROW,
        };
        let mut row = div()
            .id(self.id)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(qol_theme::SPACE_CELL))
            .w_full()
            .h(px(height))
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .rounded_none()
            .children(self.children);
        if self.kind == RowKind::Rule {
            row = row.rounded(px(qol_theme::RADIUS_CONTROL));
        }
        if self.dimmed {
            row = row.opacity(DIMMED_OPACITY);
        }
        if self.selected && self.focused {
            row = paint_settings_selection(row, self.palette);
        } else if self.attention {
            row = paint_settings_attention(row, self.palette);
        }
        if let Some(on_click) = self.on_click {
            row = row
                .cursor(gpui::CursorStyle::PointingHand)
                .hover(|style| style.bg(rgba(shared.washes.fill_hover.packed())))
                .on_click(move |event, window, cx| on_click(event, window, cx));
        }
        row
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
        .mx(px(-qol_theme::SPACE_PAD))
        .px(px(qol_theme::SPACE_PAD + qol_theme::SPACE_INSET))
        .rounded_none()
        .bg(rgb(palette.fill_current))
}

pub fn paint_rail_selection<E: Styled>(row: E, palette: SettingsPanelPalette, focused: bool) -> E {
    row.bg(rgb(if focused {
        palette.fill_current
    } else {
        palette.fill_current_quiet
    }))
}

fn paint_settings_attention<E: Styled + ParentElement>(row: E, palette: SettingsPanelPalette) -> E {
    let shared = kit();
    row.relative()
        .ml(px(-qol_theme::SPACE_PAD))
        .pl(px(qol_theme::SPACE_PAD + qol_theme::SPACE_INSET))
        .rounded_none()
        .rounded_r(px(qol_theme::RADIUS_CARD))
        .bg(rgba(shared.washes.wash_attention.packed()))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(qol_theme::SPACE_MARK))
                .bg(rgb(palette.status_warning)),
        )
}

#[derive(IntoElement)]
pub struct SettingsGroupHeader {
    title: SharedString,
    detail: Option<SharedString>,
    current: bool,
    palette: SettingsPanelPalette,
}

impl SettingsGroupHeader {
    pub fn new(
        title: impl Into<SharedString>,
        detail: Option<SharedString>,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            title: title.into(),
            detail,
            current: false,
            palette,
        }
    }

    pub fn titled(title: impl Into<SharedString>, palette: SettingsPanelPalette) -> Self {
        Self {
            title: title.into(),
            detail: None,
            current: false,
            palette,
        }
    }

    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }
}

impl RenderOnce for SettingsGroupHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let name = if self.current {
            self.palette.section_text
        } else {
            self.palette.status_muted
        };
        let detail_ink = if self.current {
            self.palette.status_accent
        } else {
            self.palette.status_muted
        };
        let block = div()
            .flex_none()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(qol_theme::SPACE_STACK))
            .pt(px(qol_theme::SPACE_PAD))
            .pb(px(qol_theme::SPACE_SNUG))
            .font_family(SharedString::from(qol_theme::font_display()))
            .font_weight(FontWeight::SEMIBOLD)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_DISPLAY))
                    .line_height(gpui::relative(1.15))
                    .text_color(rgb(name))
                    .child(SharedString::from(self.title.to_lowercase())),
            );
        let block = match self.detail {
            None => block,
            Some(detail) => block.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_NANO))
                    .line_height(gpui::relative(1.2))
                    .text_color(rgb(detail_ink))
                    .child(SharedString::from(detail.to_lowercase())),
            ),
        };
        block.child(masthead_rule().w_full().mt(px(qol_theme::SPACE_SNUG)))
    }
}

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

#[derive(IntoElement)]
pub struct SettingsSelectValue {
    text: SharedString,
    accent: Option<u32>,
    palette: SettingsPanelPalette,
}

impl SettingsSelectValue {
    pub fn new(text: impl Into<SharedString>, palette: SettingsPanelPalette) -> Self {
        Self {
            text: text.into(),
            accent: None,
            palette,
        }
    }

    pub fn accent(mut self, accent: Option<u32>) -> Self {
        self.accent = accent;
        self
    }
}

impl RenderOnce for SettingsSelectValue {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .min_w(px(FIELD_MIN_WIDTH))
            .max_w(px(VALUE_MAX_WIDTH))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .bg(rgb(self.palette.dropdown_bg))
            .text_size(px(qol_theme::TEXT_BODY))
            .text_color(rgb(self.palette.label_text))
            .children(
                self.accent
                    .map(|accent| div().flex_none().w_2().h_2().rounded_full().bg(rgb(accent))),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.text))
            .child(
                div()
                    .flex_none()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.status_muted))
                    .child("▾"),
            )
    }
}

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

#[derive(IntoElement)]
pub struct SettingsKeyCombination {
    text: SharedString,
    focused: bool,
    recording: bool,
    palette: SettingsPanelPalette,
}

impl SettingsKeyCombination {
    pub fn new(
        text: impl Into<SharedString>,
        focused: bool,
        recording: bool,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: text.into(),
            focused,
            recording,
            palette,
        }
    }
}

impl RenderOnce for SettingsKeyCombination {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        div()
            .flex_none()
            .flex()
            .items_center()
            .h(px(qol_theme::HEIGHT_INLINE))
            .px(px(qol_theme::SPACE_INSET))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .border(px(1.0))
            .border_color(rgb(if self.focused || self.recording {
                self.palette.row_border_selected
            } else {
                self.palette.panel_border
            }))
            .bg(rgba(if self.recording {
                shared.washes.wash_selected.packed()
            } else {
                alpha(self.palette.dropdown_bg, 0xff)
            }))
            .when(self.focused || self.recording, |combo| {
                combo.shadow(shared.focus_ring())
            })
            .font_family(SharedString::from(qol_theme::font_mono()))
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(if self.text.is_empty() {
                self.palette.status_muted
            } else {
                self.palette.section_text
            }))
            .child(self.text)
    }
}

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

pub fn settings_label(text: impl Into<SharedString>, palette: SettingsPanelPalette) -> gpui::Div {
    div()
        .truncate()
        .text_size(px(qol_theme::TEXT_BODY))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(palette.section_text))
        .child(text.into())
}

pub fn settings_description(
    text: impl Into<SharedString>,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    div()
        .truncate()
        .text_size(px(qol_theme::TEXT_MICRO))
        .text_color(rgb(palette.status_muted))
        .child(text.into())
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
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let value = kit().value(text);
    match tone {
        SettingsValueTone::Normal => value,
        SettingsValueTone::Muted => value.text_color(rgb(palette.status_muted)),
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
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let (background, text) = match variant {
        Some("ghost") => (rgb(palette.dropdown_bg), palette.label_text),
        Some("danger") => (rgba(alpha(palette.state_off, 0x29)), palette.state_off),
        Some("primary") | None | Some(_) => (rgb(palette.row_bg_selected), palette.section_text),
    };
    let mut control = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_TIGHT));
    if busy {
        control = control.child(settings_action_spinner(id, palette).size(px(12.)));
    }
    control
        .px(px(qol_theme::SPACE_INSET))
        .py(px(qol_theme::SPACE_TIGHT))
        .rounded(px(qol_theme::RADIUS_CONTROL))
        .when(variant == Some("ghost"), |control| {
            control.shadow(crate::kit::raised_shadow(palette.section_text))
        })
        .bg(background)
        .text_size(px(qol_theme::TEXT_CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(text))
        .child(label.into())
}

pub fn settings_dropdown_style(palette: SettingsPanelPalette) -> DropdownStyle {
    DropdownStyle {
        bg: palette.dropdown_bg,
        bg_selected: palette.fill_current,
        border: palette.row_border_selected,
        text: palette.label_text,
        text_selected: palette.section_text,
        accent: palette.row_border_selected,
    }
}

const CRUMB_MAX_WIDTH: f32 = 200.0;
const CRUMB_LINE_HEIGHT: f32 = 20.0;

pub fn settings_crumb_trail(trail: Vec<String>, palette: SettingsPanelPalette) -> gpui::Div {
    let last = trail.len().saturating_sub(1);
    let separator = rgba(crate::kit::alpha(palette.status_muted, 0x70));
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
        crumbs.push(crumb.truncate().child(label.to_lowercase()));
    }
    div()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .font_family(SharedString::from(qol_theme::font_display()))
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(qol_theme::TEXT_CAPTION))
        .line_height(px(CRUMB_LINE_HEIGHT))
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
        .gap(px(qol_theme::SPACE_STACK))
        .px(px(qol_theme::SPACE_CELL))
        .font_family(SharedString::from(qol_theme::font_display()))
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            div()
                .truncate()
                .text_size(px(qol_theme::TEXT_MASTHEAD))
                .line_height(gpui::relative(1.15))
                .text_color(rgb(kit.palette.text_primary))
                .child(SharedString::from(label.to_lowercase())),
        );
    let block = match detail {
        None => block,
        Some(detail) => block.child(
            div()
                .truncate()
                .text_size(px(qol_theme::TEXT_NANO))
                .line_height(gpui::relative(1.2))
                .text_color(rgb(if focused {
                    kit.palette.accent_ink
                } else {
                    kit.palette.text_muted
                }))
                .child(SharedString::from(detail.to_uppercase())),
        ),
    };
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

pub struct DisplayLayoutTile {
    pub connector: String,
    pub resolution: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub selected: bool,
    pub primary: bool,
    pub conflicted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayLayoutTileStyle {
    pub border_color: u32,
    pub border_width: f32,
    pub background: u32,
}

pub fn display_layout_tile_style(
    tile: &DisplayLayoutTile,
    palette: SettingsPanelPalette,
) -> DisplayLayoutTileStyle {
    let (border_color, border_width) = if tile.conflicted {
        (palette.state_off, 2.0)
    } else if tile.selected {
        (palette.row_border_selected, 2.0)
    } else {
        (palette.panel_border, 1.0)
    };
    DisplayLayoutTileStyle {
        border_color,
        border_width,
        background: if tile.selected {
            palette.row_bg_selected
        } else {
            palette.dropdown_bg
        },
    }
}

pub fn display_layout_stage(palette: SettingsPanelPalette, height: f32) -> gpui::Div {
    div()
        .relative()
        .w_full()
        .h(px(height))
        .overflow_hidden()
        .rounded(px(qol_theme::RADIUS_CARD))
        .bg(rgb(palette.dropdown_bg))
}

pub fn display_layout_tile(
    index: usize,
    tile: &DisplayLayoutTile,
    palette: SettingsPanelPalette,
) -> gpui::Stateful<gpui::Div> {
    let style = display_layout_tile_style(tile, palette);
    let mut cell = div()
        .id(("settings-display-layout-tile", index))
        .absolute()
        .left(px(tile.left))
        .top(px(tile.top))
        .w(px(tile.width))
        .h(px(tile.height))
        .rounded(px(qol_theme::RADIUS_TIGHT))
        .border(px(style.border_width))
        .border_color(rgb(style.border_color))
        .bg(rgb(style.background))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(qol_theme::SPACE_STACK))
                .px(px(qol_theme::SPACE_STACK))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(qol_theme::TEXT_MICRO))
                        .text_color(rgb(palette.label_text))
                        .child(SharedString::from(tile.connector.clone())),
                )
                .children(
                    tile.selected
                        .then(|| kit().status_pill("selected", palette.status_accent)),
                ),
        )
        .child(
            div()
                .truncate()
                .px(px(qol_theme::SPACE_STACK))
                .text_size(px(qol_theme::TEXT_NANO))
                .text_color(rgb(palette.status_muted))
                .child(SharedString::from(tile.resolution.clone())),
        );
    if tile.primary {
        cell = cell.child(
            div()
                .px(px(qol_theme::SPACE_STACK))
                .child(kit().status_pill("primary", palette.status_success)),
        );
    }
    cell
}

pub fn settings_label_group(
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(qol_theme::SPACE_STACK))
        .child(settings_label(label, palette))
        .children(description.map(|text| settings_description(text, palette)))
}

pub fn settings_message(
    text: impl Into<SharedString>,
    danger: bool,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    settings_message_frame(if danger {
        palette.status_danger
    } else {
        palette.status_muted
    })
    .child(text.into())
}

fn settings_message_frame(color: u32) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(qol_theme::TEXT_BODY))
        .text_color(rgb(color))
}

/// Spinner recipe for a query-backed value that has not answered yet.
pub fn settings_query_spinner(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Spinner {
    Spinner::new(id, rgb(palette.status_muted))
}

/// Spinner recipe for a pending action inside a settings surface.
pub fn settings_action_spinner(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Spinner {
    Spinner::new(id, rgb(palette.state_on))
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
    use super::*;

    fn palette() -> SettingsPanelPalette {
        qol_theme::settings_panel_runtime()
    }

    fn tile(selected: bool, conflicted: bool) -> DisplayLayoutTile {
        DisplayLayoutTile {
            connector: "card0-DP-1".to_string(),
            resolution: "3840x2160 @ 60 Hz".to_string(),
            left: 0.0,
            top: 0.0,
            width: 320.0,
            height: 180.0,
            selected,
            primary: false,
            conflicted,
        }
    }

    #[test]
    fn selected_tile_takes_the_accent_border_at_two_pixels() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(true, false), palette);
        assert_eq!(style.border_color, palette.row_border_selected);
        assert_eq!(style.border_width, 2.0);
        assert_ne!(style.border_color, palette.panel_border);
    }

    #[test]
    fn unselected_tile_keeps_the_panel_border_at_one_pixel() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(style.border_color, palette.panel_border);
        assert_eq!(style.border_width, 1.0);
    }

    #[test]
    fn conflicted_selected_tile_stays_on_danger_at_two_pixels() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(true, true), palette);
        assert_eq!(style.border_color, palette.state_off);
        assert_eq!(style.border_width, 2.0);
        assert_ne!(style.border_color, palette.row_border_selected);
    }

    #[test]
    fn conflicted_unselected_tile_stays_on_danger_at_two_pixels() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, true), palette);
        assert_eq!(style.border_color, palette.state_off);
        assert_eq!(style.border_width, 2.0);
        assert_eq!(style.background, palette.dropdown_bg);
    }

    #[test]
    fn background_follows_selection() {
        let palette = palette();
        let selected = display_layout_tile_style(&tile(true, false), palette);
        let unselected = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(selected.background, palette.row_bg_selected);
        assert_eq!(unselected.background, palette.dropdown_bg);
    }

    #[test]
    fn a_plain_tile_is_unchanged_from_todays_colors() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(style.border_color, palette.panel_border);
        assert_eq!(style.border_width, 1.0);
        assert_eq!(style.background, palette.dropdown_bg);
    }
}
