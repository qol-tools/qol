use gpui::prelude::*;
use gpui::{
    div, linear_color_stop, linear_gradient, point, px, rgb, rgba, Background, BoxShadow, Div,
    Rgba, SharedString,
};
use qol_hotkeys::chord::Cap;
use qol_theme::{Ground, Grounds, SystemPalette, ThemeMode, WashPalette};

use crate::icon::{icon, Icon, IconView};
use crate::key::Key;
use crate::text::{cased, TextStyled};
use qol_theme::{
    clear, translucent, Alpha, Shadow, TextStyle, FOCUS_RING_EDGE, FOCUS_RING_HALO, LINE,
    OPACITY_DISABLED, SHADOW_FLOAT, SHADOW_RAISED, STATUS_DOT, STATUS_DOT_HALO,
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

pub enum Chip {
    Key(Key),
    KeyText(SharedString),
    Count(usize, SharedString),
    Status {
        tone: u32,
        halo: u32,
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
            .shadow(float_shadow(self.palette.text_primary))
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
                    .text_color(rgb(self.palette.text_muted))
                    .child(label),
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

    pub fn row_selected_tinted_after<E: Styled + ParentElement>(
        &self,
        row: E,
        selected: bool,
        tone: u32,
        leading_width: f32,
    ) -> E {
        let colors = qol_theme::tinted_row_palette(tone, self.palette);
        let row = row
            .relative()
            .border(px(LINE))
            .border_color(rgba(0))
            .bg(rgba(
                if selected {
                    colors.selected
                } else {
                    colors.resting
                }
                .packed(),
            ));
        if !selected {
            return row;
        }
        row.child(
            div()
                .absolute()
                .left(px(leading_width - LINE))
                .right(px(-LINE))
                .top(px(-LINE))
                .bottom(px(-LINE))
                .border(px(LINE))
                .border_color(rgba(colors.selected_edge.packed())),
        )
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
            .border_color(rgba(translucent(ground.ink, Alpha::Edge)))
            .text(TextStyle::Key)
            .text_color(rgb(ground.faint))
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
                .child(self.key_name(&[key], ground.faint)),
            Chip::KeyText(text) => self.key_frame(ground).child(text),
            Chip::Count(count, label) => self
                .chip_frame(ground)
                .child(div().text_color(rgb(ground.ink)).child(count.to_string()))
                .child(label),
            Chip::Status { tone, halo, text } => self
                .chip_frame(ground)
                .child(self.status_dot(tone, halo))
                .child(text),
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
                            .text_color(rgb(ground.ink))
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
                    .text(TextStyle::ListName)
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
            .bg(rgba(self.washes.fill_hover.packed()))
            .text(TextStyle::Hint)
            .text_color(rgb(self.palette.text_secondary))
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
        self.button_base(text)
            .bg(rgb(self.palette.surface_raised))
            .text_color(rgb(self.palette.text_secondary))
            .shadow(raised_shadow(self.palette.text_primary))
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
            ActionCircleState::Resting | ActionCircleState::Disabled => self.palette.text_primary,
            ActionCircleState::Primary => self.palette.solid_ink,
            ActionCircleState::Armed => self.palette.accent_ink,
        }
    }

    pub fn action_icon(&self, glyph: Icon, state: ActionCircleState) -> IconView {
        icon(glyph, qol_theme::TEXT_BODY, self.action_ink(state))
    }

    pub fn action_circle(&self, size: ActionCircleSize, state: ActionCircleState) -> Div {
        let circle = div()
            .flex_none()
            .w(px(size.px()))
            .h(px(size.px()))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .border(px(qol_theme::LINE))
            .shadow(float_shadow(self.palette.text_primary))
            .text_color(rgb(self.action_ink(state)));
        match state {
            ActionCircleState::Resting => circle
                .bg(rgb(self.palette.surface_raised))
                .border_color(rgb(self.palette.border_subtle)),
            ActionCircleState::Primary => circle
                .bg(rgb(self.palette.accent))
                .border_color(rgb(self.palette.border_subtle)),
            ActionCircleState::Armed => circle
                .bg(rgb(self.palette.accent_fill))
                .border_color(rgb(self.palette.accent)),
            ActionCircleState::Disabled => circle
                .bg(rgb(self.palette.surface_raised))
                .border_color(rgb(self.palette.border_subtle))
                .opacity(OPACITY_DISABLED),
        }
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
                .bg(rgb(self.palette.accent))
                .text_color(rgb(self.palette.surface_raised))
                .shadow(raised_shadow(self.palette.text_primary))
        } else {
            segment.text_color(rgb(self.palette.text_secondary))
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
            .bg(rgb(self.palette.surface_hovered))
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

const TILE_TONES: [u32; 6] = [0x2f7350, 0x3a639b, 0x8a6208, 0x5c626d, 0x2f3238, 0x7a4a8a];

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
        ActionCircleSize, ActionCircleState, FOCUS_RING_EDGE, FOCUS_RING_HALO, RAIL_SCRIM_ALPHA,
        RAIL_SCRIM_END, RAIL_SCRIM_START,
    };
    use qol_theme::{ThemeMode, DARK_SYSTEM, LIGHT_SYSTEM};

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
