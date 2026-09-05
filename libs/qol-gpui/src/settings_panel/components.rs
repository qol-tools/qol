use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId, FontWeight, RenderOnce,
    SharedString, Window,
};

use crate::dropdown::DropdownStyle;
use crate::kit::{alpha, kit};
use crate::spinner::{Busy, Spinner};
use crate::theme::SettingsPanelPalette;

pub const DIMMED_OPACITY: f32 = 0.5;
const TOGGLE_TRACK_WIDTH: f32 = 40.0;
const TOGGLE_TRACK_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - 4.0;
const FIELD_MIN_WIDTH: f32 = 180.0;
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

pub fn paint_settings_selection<E: Styled + ParentElement>(
    row: E,
    palette: SettingsPanelPalette,
) -> E {
    let shared = kit();
    row.relative()
        .ml(px(-qol_theme::SPACE_PAD))
        .pl(px(qol_theme::SPACE_PAD + qol_theme::SPACE_INSET))
        .rounded_none()
        .rounded_r(px(qol_theme::RADIUS_CARD))
        .bg(rgba(shared.washes.wash_selected.packed()))
        .border(px(1.0))
        .border_color(rgba(shared.washes.hairline.packed()))
        .border_l(px(0.0))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(qol_theme::SPACE_MARK))
                .bg(rgb(palette.row_border_selected)),
        )
}

#[derive(IntoElement)]
pub struct SettingsGroupHeader {
    title: SharedString,
    count: usize,
    noun: SharedString,
    palette: SettingsPanelPalette,
}

impl SettingsGroupHeader {
    pub fn new(
        title: impl Into<SharedString>,
        count: usize,
        noun: impl Into<SharedString>,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            title: title.into(),
            count,
            noun: noun.into(),
            palette,
        }
    }
}

impl RenderOnce for SettingsGroupHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_end()
            .justify_between()
            .gap(px(qol_theme::SPACE_CELL))
            .h(px(qol_theme::HEIGHT_CONTROL))
            .ml(px(-qol_theme::SPACE_PAD))
            .mr(px(-qol_theme::SPACE_PAD))
            .pl(px(qol_theme::SPACE_INSET))
            .pr(px(qol_theme::SPACE_PAD))
            .pb(px(qol_theme::SPACE_SNUG))
            .border_b(px(1.0))
            .border_color(rgba(shared.washes.hairline.packed()))
            .bg(rgba(shared.washes.fill_resting.packed()))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.label_text))
                    .child(self.title.to_string().to_uppercase()),
            )
            .child(kit().count_chip_small(self.count, self.noun))
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
                    self.palette.state_on
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
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .bg(rgb(self.palette.dropdown_bg))
            .text_size(px(qol_theme::TEXT_BODY))
            .text_color(rgb(self.palette.label_text))
            .children(
                self.accent
                    .map(|accent| div().w_2().h_2().rounded_full().bg(rgb(accent))),
            )
            .child(self.text)
            .child(
                div()
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
        }
    }
}

impl RenderOnce for SettingsTextField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        div()
            .flex()
            .items_center()
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
            .truncate()
            .child(self.text)
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

pub fn settings_dropdown_style(palette: SettingsPanelPalette) -> DropdownStyle {
    DropdownStyle {
        bg: palette.dropdown_bg,
        bg_selected: palette.row_bg_selected,
        border: palette.row_border_selected,
        text: palette.label_text,
        text_selected: palette.section_text,
        accent: palette.row_border_selected,
    }
}

pub fn rail_caption(label: impl Into<SharedString>) -> gpui::Div {
    kit()
        .section(label)
        .h(px(qol_theme::HEIGHT_CONTROL))
        .px(px(qol_theme::SPACE_CELL))
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
