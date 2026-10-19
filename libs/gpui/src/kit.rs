use gpui::prelude::*;
use gpui::{
    div, linear_color_stop, linear_gradient, point, px, rgb, rgba, Background, BoxShadow, Div,
    Rgba, SharedString,
};
use qol_connection_health::Level;
use qol_hotkeys::chord::Cap;
use qol_theme::{Ground, Grounds, SystemPalette, ThemeMode, WashPalette};

use crate::icon::{icon, Icon};
use crate::key::Key;
use crate::text::{cased, TextStyled};
use qol_theme::{
    clear, count_disc, status_look, translucent, Alpha, Shadow, StatusTone, TextStyle,
    FOCUS_RING_EDGE, FOCUS_RING_HALO, LINE, OPACITY_DISABLED, SHADE_WELL, SHADOW_FLOAT,
    SHADOW_RAISED, STATUS_DOT, STATUS_DOT_HALO,
};

pub const HEADER_HEIGHT: f32 = qol_theme::HEIGHT_BAND;
pub const SECTION_HEIGHT: f32 = qol_theme::HEIGHT_INLINE;
pub const GUTTER: f32 = qol_theme::SPACE_GUTTER;
pub const ROW_METADATA_WIDTH: f32 = 48.0;

#[derive(Clone, Copy)]
pub enum WindowControlIcon {
    Collapse,
    Expand,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionCircleSize {
    Full,
    Control,
    Inline,
}

impl ActionCircleSize {
    pub fn px(self) -> f32 {
        match self {
            Self::Full => qol_theme::ACTION_CIRCLE_SIZE,
            Self::Control => qol_theme::HEIGHT_CONTROL,
            Self::Inline => qol_theme::HEIGHT_INLINE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionCircleState {
    Resting,
    Primary,
    Armed,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionCircleGround {
    Pane,
    Picture,
}

const ACTION_CIRCLE_GROUP: &str = "action-circle";

pub const SECTION_MARK_WIDTH: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Attention,
    Invalid,
    Done,
    Quiet,
}

pub const CHIP_HEIGHT: f32 = 22.0;
pub const KEY_CHIP_HEIGHT: f32 = 20.0;
pub const CHIP_COUNT_HEIGHT: f32 = 16.0;

pub enum Chip {
    Key(Key),
    KeyText(SharedString),
    Count(usize, SharedString),
    Status {
        tone: StatusTone,
        text: SharedString,
    },
    Tag(SharedString),
}

#[derive(Clone, Copy)]
pub struct Kit {
    pub palette: SystemPalette,
    pub washes: WashPalette,
    pub grounds: Grounds,
}

impl Kit {
    pub fn new(mode: ThemeMode, palette: SystemPalette) -> Self {
        Self {
            palette,
            washes: WashPalette::for_mode(mode, palette),
            grounds: Grounds::from_theme(mode, palette),
        }
    }

    pub fn connection_hue(&self, level: Level) -> u32 {
        match level {
            Level::Unknown => self.grounds.pane.faint,
            Level::Good => self.palette.success,
            Level::Slow => self.palette.warning,
            Level::Bad => self.palette.danger,
        }
    }

    pub fn focus_ring(&self, ground: Ground) -> Vec<BoxShadow> {
        vec![
            BoxShadow {
                color: rgb(ground.mark).into(),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(FOCUS_RING_EDGE),
            },
            BoxShadow {
                color: rgba(ground.halo.packed()).into(),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(FOCUS_RING_HALO),
            },
        ]
    }

    pub fn window(&self) -> Div {
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .rounded_none()
            .bg(rgb(self.grounds.pane.bg))
            .border(px(LINE))
            .border_color(rgba(self.grounds.pane.edge.packed()))
            .shadow(float_shadow(self.washes.cast.rgb))
    }

    pub fn heading(&self, title: impl Into<SharedString>, colophon: Option<SharedString>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_CELL))
            .h(px(HEADER_HEIGHT))
            .px(px(GUTTER))
            .child(self.heading_title(TextStyle::Heading, title, colophon, true))
    }

    pub fn heading_title(
        &self,
        style: TextStyle,
        title: impl Into<SharedString>,
        colophon: Option<SharedString>,
        live: bool,
    ) -> Div {
        let pane = self.grounds.pane;
        let (title_ink, colophon_ink) = if live {
            (pane.ink, self.palette.accent_ink)
        } else {
            (pane.faint, pane.faint)
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_STACK))
            .child(
                div()
                    .text(style)
                    .text_color(rgb(title_ink))
                    .child(title.into()),
            )
            .when_some(colophon, |block, colophon| {
                block.child(
                    div()
                        .text(TextStyle::Colophon)
                        .text_color(rgb(colophon_ink))
                        .child(colophon),
                )
            })
    }

    pub fn section(&self, label: impl Into<SharedString>) -> Div {
        let label = cased(TextStyle::Label, &label.into());
        div()
            .flex_none()
            .flex()
            .items_center()
            .w_full()
            .h(px(SECTION_HEIGHT))
            .px(px(GUTTER))
            .gap(px(qol_theme::SPACE_SNUG))
            .child(
                div()
                    .flex_none()
                    .w(px(SECTION_MARK_WIDTH))
                    .h(px(2.0))
                    .bg(rgb(self.palette.accent)),
            )
            .child(
                div()
                    .text(TextStyle::Label)
                    .text_color(rgb(self.palette.text_secondary))
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(LINE))
                    .bg(rgba(self.washes.hairline.packed())),
            )
    }

    pub fn row_compact_described(&self) -> Div {
        self.row_of_height(qol_theme::LIST_ENTRY_HEIGHTS[1])
            .h(px(qol_theme::LIST_ENTRY_HEIGHTS[1]))
            .py(px(qol_theme::SPACE_TIGHT))
    }

    fn row_of_height(&self, height: f32) -> Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(qol_theme::SPACE_CELL))
            .w_full()
            .min_h(px(height))
            .px(px(GUTTER))
            .rounded_none()
    }

    pub fn pointable<E: Styled + InteractiveElement>(&self, element: E, lift: Rgba) -> E {
        element.hover(move |style| style.bg(lift))
    }

    pub fn revealed_by<E: Styled + InteractiveElement>(
        &self,
        element: E,
        group: &'static str,
    ) -> E {
        element
            .opacity(0.0)
            .group_hover(group, |style| style.opacity(1.0))
    }

    pub fn highlight_ground(&self, selected: bool) -> Ground {
        if selected {
            self.grounds.band
        } else {
            self.grounds.pane
        }
    }

    pub fn highlight<E: Styled + InteractiveElement>(&self, row: E, selected: bool) -> E {
        let row = self.pointable(
            row.rounded_none(),
            rgb(self.highlight_ground(selected).lift),
        );
        if selected {
            row.bg(rgb(self.grounds.band.bg))
        } else {
            row
        }
    }

    pub fn value(&self, text: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .text(TextStyle::Value)
            .text_color(rgb(self.palette.text_secondary))
            .child(text.into())
    }

    fn key_frame(&self, ground: Ground) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .h(px(KEY_CHIP_HEIGHT))
            .px(px(qol_theme::SPACE_SNUG))
            .rounded(px(qol_theme::RADIUS_KEYCAP))
            .border(px(qol_theme::LINE))
            .border_color(rgba(ground.edge.packed()))
            .bg(rgba(ground.well.packed()))
            .shadow(key_cast())
            .text(TextStyle::Key)
            .text_color(rgb(ground.soft))
    }

    fn chip_frame(&self, ground: Ground) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .h(px(CHIP_HEIGHT))
            .px(px(qol_theme::SPACE_INSET))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .bg(rgba(ground.well.packed()))
            .text(TextStyle::Detail)
            .text_color(rgb(ground.soft))
    }

    pub fn chip(&self, chip: Chip, ground: Ground) -> Div {
        match chip {
            Chip::Key(key) => self
                .key_frame(ground)
                .child(self.key_name(&[key], ground.soft)),
            Chip::KeyText(text) => self.key_frame(ground).child(text),
            Chip::Count(count, label) => {
                let inset = px((CHIP_HEIGHT - CHIP_COUNT_HEIGHT) / 2.0);
                let chip = self.chip_frame(ground).pl(inset).child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(px(CHIP_COUNT_HEIGHT))
                        .h(px(CHIP_COUNT_HEIGHT))
                        .px(px(qol_theme::SPACE_TIGHT))
                        .rounded_full()
                        .bg(rgb(count_disc(ground)))
                        .text(TextStyle::Label)
                        .text_color(rgb(ground.on_mark))
                        .child(count.to_string()),
                );
                if label.is_empty() {
                    chip.pr(inset)
                } else {
                    chip.child(label)
                }
            }
            Chip::Status { tone, text } => {
                let look = status_look(tone, ground, self.palette);
                self.chip_frame(ground)
                    .bg(rgba(look.wash.packed()))
                    .text_color(rgb(look.ink))
                    .child(self.status_dot(look.dot, clear(look.dot)))
                    .child(text)
            }
            Chip::Tag(text) => self.chip_frame(ground).child(text),
        }
    }

    pub fn notice(
        &self,
        tone: NoticeTone,
        title: impl Into<SharedString>,
        detail: Option<gpui::AnyElement>,
    ) -> Div {
        let (ground, dot, halo) = match tone {
            NoticeTone::Attention => (
                self.grounds.attention,
                self.palette.warning,
                self.washes.halo_attention,
            ),
            NoticeTone::Invalid => (
                self.grounds.invalid,
                self.palette.danger,
                self.washes.halo_invalid,
            ),
            NoticeTone::Done => (
                self.grounds.pane,
                self.palette.success,
                self.washes.halo_success,
            ),
            NoticeTone::Quiet => (
                self.grounds.pane,
                self.palette.text_muted,
                self.washes.fill_resting,
            ),
        };
        let title_ink = match tone {
            NoticeTone::Attention | NoticeTone::Invalid => ground.mark,
            NoticeTone::Done | NoticeTone::Quiet => ground.ink,
        };
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_CELL))
            .px(px(qol_theme::SPACE_GUTTER))
            .py(px(qol_theme::SPACE_CELL))
            .bg(rgb(ground.bg))
            .child(self.status_dot(dot, halo.packed()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(qol_theme::SPACE_STACK))
                    .child(
                        div()
                            .text(TextStyle::ListName)
                            .text_color(rgb(title_ink))
                            .child(title.into()),
                    )
                    .when_some(detail, |block, detail| {
                        block.child(
                            div()
                                .text(TextStyle::Detail)
                                .line_clamp(2)
                                .text_color(rgb(ground.soft))
                                .child(detail),
                        )
                    }),
            )
    }

    pub fn empty(&self, title: impl Into<SharedString>, detail: Option<SharedString>) -> Div {
        let pane = self.grounds.pane;
        div()
            .flex_1()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(qol_theme::SPACE_STACK))
            .px(px(qol_theme::SPACE_GUTTER))
            .child(
                div()
                    .text(TextStyle::Name)
                    .text_color(rgb(pane.ink))
                    .child(title.into()),
            )
            .when_some(detail, |block, detail| {
                block.child(
                    div()
                        .text(TextStyle::Detail)
                        .text_color(rgb(pane.soft))
                        .child(detail),
                )
            })
    }

    pub fn keycap_inked(&self, key: Key, ink: u32) -> Div {
        self.key_frame(self.grounds.pane)
            .text_color(rgb(ink))
            .child(self.key_name(&[key], ink))
    }

    pub fn key_name(&self, keys: &[Key], ink: u32) -> Div {
        let size = TextStyle::Key.spec().size;
        let mut name = div()
            .flex_none()
            .flex()
            .items_center()
            .text(TextStyle::Key)
            .text_color(rgb(ink));
        for (index, key) in keys.iter().enumerate() {
            if index > 0 {
                name = name.child(" / ");
            }
            for part in key.caps() {
                name = match part {
                    Cap::Text(text) => name.child(text),
                    Cap::Glyph(glyph) => name.child(icon(glyph.into(), size, ink)),
                };
            }
        }
        name
    }

    pub fn status_dot(&self, tone: u32, halo: u32) -> Div {
        div()
            .flex_none()
            .size(px(STATUS_DOT))
            .rounded_full()
            .bg(rgb(tone))
            .shadow(vec![BoxShadow {
                color: rgba(halo).into(),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(STATUS_DOT_HALO),
            }])
    }

    pub fn live_dot(
        &self,
        id: impl Into<gpui::ElementId>,
        tone: u32,
        halo: u32,
        live: bool,
    ) -> gpui::AnyElement {
        let dot = self.status_dot(tone, halo);
        if live {
            crate::status_indicator::pulse_dot(dot, id.into())
        } else {
            dot.into_any_element()
        }
    }

    pub fn hint_bar(&self) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_GUTTER))
            .w_full()
            .h(px(qol_theme::HEIGHT_HINT_BAR))
            .px(px(qol_theme::SPACE_PAD))
            .border_t(px(qol_theme::LINE))
            .border_color(rgba(self.washes.hairline.packed()))
            .text(TextStyle::Hint)
            .text_color(rgb(self.grounds.pane.faint))
    }

    pub fn hint(&self, key: Key, label: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .child(self.chip(Chip::Key(key), self.grounds.pane))
            .child(label.into())
    }

    pub fn hint_button(
        &self,
        id: impl Into<gpui::ElementId>,
        key: Key,
        label: impl Into<SharedString>,
        on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> gpui::Stateful<Div> {
        self.pointable(
            self.hint(key, label)
                .id(id)
                .px(px(qol_theme::SPACE_SNUG))
                .cursor(gpui::CursorStyle::PointingHand)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |event, window, cx| {
                    if event.click_count() <= 1 {
                        on_click(event, window, cx);
                    }
                }),
            rgba(self.washes.fill_hover.packed()),
        )
    }

    pub fn letter_tile(&self, name: &str) -> Div {
        let glyph = name
            .chars()
            .find(|character| character.is_alphanumeric())
            .map(|character| character.to_uppercase().to_string())
            .unwrap_or_else(|| "\u{2022}".to_string());
        div()
            .flex_none()
            .w(px(23.0))
            .h(px(23.0))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(tile_tone(name)))
            .text_color(rgb(0xffffff))
            .text(TextStyle::Label)
            .child(glyph)
    }

    pub fn vertical_identity_tab(&self, text: impl Into<SharedString>, tone: u32) -> Div {
        div()
            .absolute()
            .left(px(-LINE))
            .top(px(-LINE))
            .bottom(px(-LINE))
            .w(px(crate::vertical_label::WIDTH))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(tone))
            .child(crate::vertical_label::VerticalLabel::new(text))
    }

    pub fn scroll_cue(
        &self,
        source: crate::scrollbar::ScrollSource,
        ground: Ground,
    ) -> impl IntoElement {
        crate::scrollbar::scroll_cue(source, ground)
    }

    pub fn row_metadata(&self) -> Div {
        div()
            .flex_none()
            .w(px(ROW_METADATA_WIDTH))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(qol_theme::SPACE_STACK))
    }

    pub fn row_separator(&self) -> Div {
        div()
            .absolute()
            .bottom_0()
            .left(px(qol_theme::SPACE_PAD))
            .right(px(qol_theme::SPACE_PAD))
            .h(px(1.0))
            .bg(rgba(self.washes.separator.packed()))
    }

    pub fn button_ghost(&self, text: impl Into<SharedString>) -> Div {
        self.lifted_well(self.button_base(text))
            .text_color(rgb(self.palette.text_primary))
    }

    fn lifted_well(&self, control: Div) -> Div {
        let hover = rgba(translucent(self.palette.text_primary, Alpha::Tint));
        control
            .bg(rgba(self.grounds.pane.well.packed()))
            .hover(move |style| style.bg(hover))
    }

    pub fn window_control(&self, icon: WindowControlIcon) -> Div {
        let ink = rgb(self.palette.text_secondary);
        let hover = rgba(self.washes.fill_hover.packed());
        div()
            .group("window-control")
            .flex_none()
            .size(px(qol_theme::HEIGHT_INLINE))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .bg(rgba(0))
            .child(
                div()
                    .size(px(qol_theme::SPACE_GUTTER))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(qol_theme::RADIUS_TIGHT))
                    .group_hover("window-control", move |style| style.bg(hover))
                    .child(
                        gpui::canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| {
                                let mut path = gpui::PathBuilder::stroke(px(1.5));
                                let left = bounds.left() + px(1.0);
                                let right = bounds.right() - px(1.0);
                                let top = bounds.top() + px(1.0);
                                let bottom = bounds.bottom() - px(1.0);
                                match icon {
                                    WindowControlIcon::Collapse | WindowControlIcon::Expand => {
                                        let center = bounds.center();
                                        let offset = if matches!(icon, WindowControlIcon::Expand) {
                                            px(-2.0)
                                        } else {
                                            px(2.0)
                                        };
                                        path.move_to(point(left, center.y + offset));
                                        path.line_to(point(center.x, center.y - offset));
                                        path.line_to(point(right, center.y + offset));
                                    }
                                    WindowControlIcon::Close => {
                                        path.move_to(point(left, top));
                                        path.line_to(point(right, bottom));
                                        path.move_to(point(right, top));
                                        path.line_to(point(left, bottom));
                                    }
                                }
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, ink);
                                }
                            },
                        )
                        .size(px(qol_theme::SPACE_CELL)),
                    ),
            )
    }

    fn button_base(&self, text: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .px(px(qol_theme::SPACE_CELL))
            .py(px(qol_theme::SPACE_SNUG))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .text(TextStyle::ListName)
            .child(text.into())
    }

    pub fn action_ink(&self, state: ActionCircleState) -> u32 {
        match state {
            ActionCircleState::Resting | ActionCircleState::Disabled => self.palette.text_secondary,
            ActionCircleState::Primary => self.palette.solid_ink,
            ActionCircleState::Armed => self.palette.accent_ink,
        }
    }

    pub fn action_icon(&self, glyph: Icon, state: ActionCircleState) -> Div {
        let size = qol_theme::TEXT_BODY;
        let resting = div().child(icon(glyph, size, self.action_ink(state)));
        let frame = div().flex_none().relative().size(px(size));
        if state != ActionCircleState::Resting {
            return frame.child(resting);
        }
        let hovered =
            div()
                .absolute()
                .inset_0()
                .child(icon(glyph, size, self.palette.text_primary));
        frame
            .child(resting.group_hover(ACTION_CIRCLE_GROUP, |style| style.opacity(0.0)))
            .child(self.revealed_by(hovered, ACTION_CIRCLE_GROUP))
    }

    pub fn action_circle(
        &self,
        size: ActionCircleSize,
        state: ActionCircleState,
        ground: ActionCircleGround,
    ) -> Div {
        let circle = div()
            .group(ACTION_CIRCLE_GROUP)
            .flex_none()
            .w(px(size.px()))
            .h(px(size.px()))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center();
        match (state, ground) {
            (ActionCircleState::Resting, ActionCircleGround::Pane) => self.lifted_well(circle),
            (ActionCircleState::Resting, ActionCircleGround::Picture) => {
                self.pointable(self.picture_disc(circle), rgb(self.grounds.menu.lift))
            }
            (ActionCircleState::Primary, _) => {
                let mut hovered = vec![BoxShadow {
                    color: rgba(self.grounds.pane.halo.packed()).into(),
                    offset: point(px(0.0), px(0.0)),
                    blur_radius: px(0.0),
                    spread_radius: px(qol_theme::ACTION_CIRCLE_HOVER_HALO),
                }];
                hovered.extend(float_shadow(self.palette.text_primary));
                circle
                    .bg(rgb(self.palette.accent))
                    .border(px(qol_theme::LINE))
                    .border_color(rgb(self.palette.border_subtle))
                    .shadow(float_shadow(self.palette.text_primary))
                    .hover(move |style| style.shadow(hovered))
            }
            (ActionCircleState::Armed, _) => circle
                .bg(rgb(self.palette.accent_fill))
                .border(px(qol_theme::LINE))
                .border_color(rgb(self.palette.accent))
                .shadow(float_shadow(self.palette.text_primary)),
            (ActionCircleState::Disabled, ActionCircleGround::Pane) => circle
                .bg(rgba(self.grounds.pane.well.packed()))
                .opacity(OPACITY_DISABLED),
            (ActionCircleState::Disabled, ActionCircleGround::Picture) => {
                self.picture_disc(circle).opacity(OPACITY_DISABLED)
            }
        }
    }

    fn picture_disc(&self, circle: Div) -> Div {
        circle
            .bg(rgb(self.grounds.menu.bg))
            .border(px(qol_theme::LINE))
            .border_color(rgb(self.palette.border_subtle))
            .shadow(float_shadow(self.palette.text_primary))
    }

    pub fn action_row(&self) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(qol_theme::ACTION_CIRCLE_GAP))
    }

    pub fn segment(&self, label: impl Into<SharedString>, active: bool) -> Div {
        let segment = div()
            .px(px(qol_theme::SPACE_CELL))
            .py(px(qol_theme::SPACE_STACK))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .text(TextStyle::ListName)
            .child(label.into());
        if active {
            segment
                .bg(rgb(self.grounds.band.bg))
                .text_color(rgb(self.palette.text_primary))
        } else {
            let hover = rgba(self.washes.fill_hover.packed());
            let ink = rgb(self.palette.text_primary);
            segment
                .text_color(rgb(self.grounds.pane.faint))
                .hover(move |style| style.bg(hover).text_color(ink))
        }
    }

    pub fn segmented_group(&self) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_row()
            .gap(px(qol_theme::SPACE_STACK))
            .p(px(qol_theme::SPACE_STACK))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .bg(rgba(self.grounds.pane.well.packed()))
    }
}

/// The index `delta` steps from `current` in a row of `count` entries,
/// wrapping at both ends.
pub fn wrap_index(current: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        return current;
    }
    let count = count as isize;
    (((current as isize + delta) % count + count) % count) as usize
}

/// The next index a keyboard selection may land on, `delta` steps from
/// `current`, stepping over every entry the row has disabled. Stays put when
/// the row has nothing selectable, so a disabled control never holds the
/// selection.
pub fn next_selectable(
    current: usize,
    delta: isize,
    count: usize,
    enabled: impl Fn(usize) -> bool,
) -> usize {
    let mut candidate = current;
    for _ in 0..count {
        candidate = wrap_index(candidate, delta, count);
        if enabled(candidate) {
            return candidate;
        }
    }
    current
}

/// The state a circle takes inside a keyboard-navigable action row. The
/// selected circle is the row's only accent, so no other circle can read as
/// selected while the selection sits elsewhere.
pub fn row_circle_state(enabled: bool, selected: bool) -> ActionCircleState {
    match (enabled, selected) {
        (false, _) => ActionCircleState::Disabled,
        (true, true) => ActionCircleState::Armed,
        (true, false) => ActionCircleState::Resting,
    }
}

pub fn action_row_width(count: usize, size: ActionCircleSize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    count as f32 * size.px() + (count - 1) as f32 * qol_theme::ACTION_CIRCLE_GAP
}

pub fn key_cast() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: rgba(SHADE_WELL).into(),
        offset: point(px(0.0), px(LINE)),
        blur_radius: px(0.0),
        spread_radius: px(0.0),
    }]
}

pub fn float_shadow(ink: u32) -> Vec<BoxShadow> {
    shadow(SHADOW_FLOAT, ink)
}

pub fn raised_shadow(ink: u32) -> Vec<BoxShadow> {
    shadow(SHADOW_RAISED, ink)
}

fn shadow(layers: Shadow, ink: u32) -> Vec<BoxShadow> {
    layers
        .iter()
        .map(|layer| BoxShadow {
            color: rgba(translucent(ink, layer.alpha)).into(),
            offset: point(px(0.0), px(f32::from(layer.y))),
            blur_radius: px(f32::from(layer.blur)),
            spread_radius: px(0.0),
        })
        .collect()
}

pub const RAIL_SCRIM_START: f32 = 0.32;
pub const RAIL_SCRIM_END: f32 = 0.5;
pub const RAIL_SCRIM_ALPHA: Alpha = Alpha::Veil;

pub fn rail_scrim(surface: u32) -> Background {
    linear_gradient(
        90.0,
        linear_color_stop(rgba(clear(surface)), RAIL_SCRIM_START),
        linear_color_stop(rgba(translucent(surface, RAIL_SCRIM_ALPHA)), RAIL_SCRIM_END),
    )
}

const TILE_TONES: [u32; 6] = [0x2f7350, 0x3a639b, 0x8a6208, 0x5c626d, 0x41444c, 0x7a4a8a];

fn tile_tone(name: &str) -> u32 {
    let hash = name.bytes().fold(0u32, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte.into())
    });
    TILE_TONES[(hash % TILE_TONES.len() as u32) as usize]
}

pub fn path_label(path: &str) -> (String, String) {
    let body = path.strip_suffix('/').unwrap_or(path);
    if body.is_empty() {
        return (String::new(), path.to_string());
    }
    let bytes = body.as_bytes();
    let starts: Vec<usize> = (0..bytes.len())
        .filter(|i| (*i == 0 || bytes[i - 1] == b'/') && bytes[*i] != b'/')
        .collect();
    if starts.len() < 2 {
        return (String::new(), path.to_string());
    }
    let cut = starts[starts.len() - 2];
    (body[..cut].to_string(), body[cut..].to_string())
}

pub fn kit() -> Kit {
    let theme = qol_theme::runtime_theme();
    let kit = Kit::new(theme.mode, theme.system);
    if qol_theme::window_is_quiet() {
        Kit {
            grounds: kit.grounds.quiet(),
            ..kit
        }
    } else {
        kit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowLook {
    Live,
    Quiet,
}

pub fn enter_window(look: WindowLook) {
    qol_theme::set_window_quiet(look == WindowLook::Quiet);
}

#[cfg(test)]
mod tests {
    use super::Kit;
    use super::{
        action_row_width, next_selectable, path_label, rail_scrim, row_circle_state,
        ActionCircleGround, ActionCircleSize, ActionCircleState, Chip, CHIP_COUNT_HEIGHT,
        CHIP_HEIGHT, FOCUS_RING_EDGE, FOCUS_RING_HALO, RAIL_SCRIM_ALPHA, RAIL_SCRIM_END,
        RAIL_SCRIM_START,
    };
    use gpui::{px, rgb, rgba, Fill, Hsla, SharedString, Styled};
    use qol_theme::{status_look, StatusTone, ThemeMode, DARK_SYSTEM, LIGHT_SYSTEM};

    #[test]
    fn a_circle_over_a_picture_keeps_an_opaque_edged_disc() {
        for (mode, palette) in [
            (ThemeMode::Light, LIGHT_SYSTEM),
            (ThemeMode::Dark, DARK_SYSTEM),
        ] {
            let kit = Kit::new(mode, palette);
            for state in [ActionCircleState::Resting, ActionCircleState::Disabled] {
                let mut picture = kit.action_circle(
                    ActionCircleSize::Control,
                    state,
                    ActionCircleGround::Picture,
                );
                let style = picture.style();
                assert_eq!(
                    style.background,
                    Some(Fill::from(rgb(kit.grounds.menu.bg))),
                    "{state:?}"
                );
                assert_eq!(
                    style.border_color,
                    Some(Hsla::from(rgb(palette.border_subtle))),
                    "{state:?}"
                );
                assert!(
                    style
                        .box_shadow
                        .as_ref()
                        .is_some_and(|shadow| !shadow.is_empty()),
                    "{state:?}"
                );
                let mut pane =
                    kit.action_circle(ActionCircleSize::Control, state, ActionCircleGround::Pane);
                assert_eq!(
                    pane.style().background,
                    Some(Fill::from(rgba(kit.grounds.pane.well.packed()))),
                    "{state:?}"
                );
                assert_eq!(pane.style().border_color, None, "{state:?}");
            }
        }
    }

    #[test]
    fn a_danger_status_chip_sits_on_its_own_wash() {
        for (mode, palette) in [
            (ThemeMode::Light, LIGHT_SYSTEM),
            (ThemeMode::Dark, DARK_SYSTEM),
        ] {
            let kit = Kit::new(mode, palette);
            let ground = kit.grounds.pane;
            let mut chip = kit.chip(
                Chip::Status {
                    tone: StatusTone::Danger,
                    text: "offline".into(),
                },
                ground,
            );
            let wash = status_look(StatusTone::Danger, ground, palette).wash;
            assert_eq!(wash.rgb, palette.danger);
            assert_eq!(
                chip.style().background,
                Some(Fill::from(rgba(wash.packed())))
            );
        }
    }

    #[test]
    fn a_count_without_a_label_closes_the_pill_round_the_disc() {
        let kit = Kit::new(ThemeMode::Dark, DARK_SYSTEM);
        let inset = px((CHIP_HEIGHT - CHIP_COUNT_HEIGHT) / 2.0);
        let mut bare = kit.chip(Chip::Count(2, SharedString::default()), kit.grounds.pane);
        assert_eq!(bare.style().padding.left, Some(inset.into()));
        assert_eq!(bare.style().padding.right, Some(inset.into()));
        let mut labelled = kit.chip(Chip::Count(2, "live".into()), kit.grounds.pane);
        assert_eq!(
            labelled.style().padding.right,
            Some(px(qol_theme::SPACE_INSET).into())
        );
    }

    #[test]
    fn the_selection_steps_over_disabled_entries() {
        // Undo and Redo sit at 1 and 2 and are disabled with an empty history.
        let enabled = |index: usize| !matches!(index, 1 | 2);
        let cases = [
            (0, 1, 3, "forward over both"),
            (3, -1, 0, "back over both"),
            (0, -1, 5, "back wraps to the last enabled"),
            (5, 1, 0, "forward wraps to the first enabled"),
        ];
        for (current, delta, expected, note) in cases {
            assert_eq!(
                next_selectable(current, delta, 6, enabled),
                expected,
                "{note}"
            );
        }
    }

    #[test]
    fn unknown_is_neutral_and_measured_levels_take_the_status_hues() {
        use qol_connection_health::Level;
        for (mode, palette) in [
            (ThemeMode::Dark, DARK_SYSTEM),
            (ThemeMode::Light, LIGHT_SYSTEM),
        ] {
            let kit = Kit::new(mode, palette);
            assert_eq!(kit.connection_hue(Level::Unknown), kit.grounds.pane.faint);
            assert_ne!(kit.connection_hue(Level::Unknown), palette.success);
            assert_eq!(kit.connection_hue(Level::Good), palette.success);
            assert_eq!(kit.connection_hue(Level::Slow), palette.warning);
            assert_eq!(kit.connection_hue(Level::Bad), palette.danger);
        }
    }

    #[test]
    fn a_row_with_nothing_selectable_keeps_its_selection() {
        assert_eq!(next_selectable(2, 1, 6, |_| false), 2);
        assert_eq!(next_selectable(2, 1, 0, |_| true), 2);
    }

    #[test]
    fn only_the_selected_circle_in_a_row_carries_an_accent() {
        let cases = [
            (true, true, ActionCircleState::Armed),
            (true, false, ActionCircleState::Resting),
            (false, false, ActionCircleState::Disabled),
            (false, true, ActionCircleState::Disabled),
        ];
        for (enabled, selected, expected) in cases {
            assert_eq!(
                row_circle_state(enabled, selected),
                expected,
                "enabled: {enabled} selected: {selected}"
            );
        }
    }

    #[test]
    fn the_focus_ring_keeps_a_solid_inner_edge_inside_a_soft_halo() {
        for (mode, palette) in [
            (ThemeMode::Light, LIGHT_SYSTEM),
            (ThemeMode::Dark, DARK_SYSTEM),
        ] {
            let kit = Kit::new(mode, palette);
            for (ground, mark) in [
                (kit.grounds.pane, palette.accent),
                (kit.grounds.band, palette.text_primary),
            ] {
                let ring = kit.focus_ring(ground);
                assert_eq!(ring.len(), 2);

                let edge = &ring[0];
                assert_eq!(f32::from(edge.spread_radius), FOCUS_RING_EDGE);
                assert_eq!(f32::from(edge.blur_radius), 0.0);
                assert_eq!(edge.color.a, 1.0);
                assert_eq!(edge.color, gpui::Hsla::from(gpui::rgb(mark)));

                let halo = &ring[1];
                assert_eq!(f32::from(halo.spread_radius), FOCUS_RING_HALO);
                assert!(halo.color.a > 0.0 && halo.color.a < 1.0);
                assert!(halo.spread_radius > edge.spread_radius);
            }
        }
    }

    #[test]
    fn a_quiet_window_rings_focus_without_a_halo() {
        let quiet = Kit::new(ThemeMode::Light, LIGHT_SYSTEM).grounds.quiet();
        let ring = Kit::new(ThemeMode::Light, LIGHT_SYSTEM).focus_ring(quiet.pane);
        assert_eq!(ring[1].color.a, 0.0);
    }

    #[test]
    fn the_rail_scrim_stays_clear_through_a_third_then_ramps_to_full_alpha() {
        let rendered = format!("{:?}", rail_scrim(0x000000));
        assert!(rendered.contains("LinearGradient(90"));
        assert!(rendered.contains(&format!("percentage: {RAIL_SCRIM_START}")));
        assert!(rendered.contains(&format!("percentage: {RAIL_SCRIM_END}")));
        assert!(rendered.contains("a: 0.0"));
        assert!(rendered.contains(&format!(
            "a: {}",
            f32::from(RAIL_SCRIM_ALPHA.byte()) / 255.0
        )));
    }

    #[test]
    fn action_row_width_sums_circles_and_gaps() {
        assert_eq!(action_row_width(0, ActionCircleSize::Full), 0.0);
        assert_eq!(action_row_width(1, ActionCircleSize::Full), 46.0);
        assert_eq!(
            action_row_width(5, ActionCircleSize::Full),
            5.0 * 46.0 + 4.0 * 14.0
        );
        assert_eq!(
            action_row_width(2, ActionCircleSize::Control),
            2.0 * 36.0 + 14.0
        );
        assert_eq!(action_row_width(1, ActionCircleSize::Inline), 28.0);
    }

    #[test]
    fn path_label_splits_a_deep_absolute_path_keeping_the_last_two_components() {
        assert_eq!(
            path_label("/home/kmrh47/Pictures/qol/204118.png"),
            (
                "/home/kmrh47/Pictures/".to_string(),
                "qol/204118.png".to_string()
            )
        );
    }

    #[test]
    fn path_label_holds_the_root_in_head_for_a_two_component_path() {
        assert_eq!(
            path_label("/tmp/a.png"),
            ("/".to_string(), "tmp/a.png".to_string())
        );
    }

    #[test]
    fn path_label_returns_a_single_component_verbatim() {
        assert_eq!(path_label("a.png"), (String::new(), "a.png".to_string()));
    }

    #[test]
    fn path_label_collapses_an_empty_string_to_two_empty_halves() {
        assert_eq!(path_label(""), (String::new(), String::new()));
    }

    #[test]
    fn path_label_treats_a_trailing_separator_as_absent() {
        assert_eq!(
            path_label("/a/b/c/"),
            ("/a/".to_string(), "b/c".to_string())
        );
    }
}
