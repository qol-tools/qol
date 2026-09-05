use gpui::prelude::*;
use gpui::{
    div, linear_color_stop, linear_gradient, point, px, rgb, rgba, Background, BoxShadow, Div,
    FontWeight, SharedString,
};
use qol_theme::{SystemPalette, ThemeMode, WashPalette};

pub const FLOAT_SHADOW_OFFSET: f32 = 2.0;
pub const FLOAT_SHADOW_ALPHA: u8 = 0x1a;

pub const DISABLED_OPACITY: f32 = 0.4;

pub const HEADER_HEIGHT: f32 = qol_theme::HEIGHT_BAND;
pub const SECTION_HEIGHT: f32 = qol_theme::HEIGHT_INLINE;
pub const ROW_HEIGHT: f32 = qol_theme::HEIGHT_SETTING_ROW;
pub const ROW_DESCRIBED_HEIGHT: f32 = qol_theme::HEIGHT_SETTING_ROW;
pub const ROW_TIGHT_HEIGHT: f32 = 32.0;
pub const GUTTER: f32 = qol_theme::SPACE_GUTTER;
pub const LAMP_SIZE: f32 = 10.0;
pub const ROW_METADATA_WIDTH: f32 = 48.0;
pub const ROW_BORDER_WIDTH: f32 = 1.0;

pub const FOCUS_RING_EDGE: f32 = 1.5;
pub const FOCUS_RING_HALO: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowState {
    #[default]
    Resting,
    Hover,
    Current,
    NeedsAttention,
    Invalid,
    Disabled,
}

#[derive(Clone, Copy)]
pub enum WindowControlIcon {
    Collapse,
    Expand,
    Close,
}

#[derive(Clone, Copy)]
pub struct Kit {
    pub palette: SystemPalette,
    pub washes: WashPalette,
}

impl Kit {
    pub fn new(mode: ThemeMode, palette: SystemPalette) -> Self {
        Self {
            palette,
            washes: WashPalette::for_mode(mode, palette),
        }
    }

    pub fn focus_ring(&self) -> Vec<BoxShadow> {
        focus_ring_from(self.palette.accent, self.washes.accent_halo.packed())
    }

    pub fn panel(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .rounded(px(qol_theme::RADIUS_WINDOW))
            .overflow_hidden()
            .shadow(float_shadow(self.palette.text_primary))
            .bg(rgb(self.palette.surface_elevated))
    }

    pub fn header(&self, title: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .h(px(HEADER_HEIGHT))
            .px(px(GUTTER))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.text_primary))
                    .child(title.into()),
            )
    }

    pub fn section(&self, label: impl Into<SharedString>) -> Div {
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
                    .w(px(10.0))
                    .h(px(2.0))
                    .bg(rgb(self.palette.accent)),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_MICRO))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.text_muted))
                    .child(label.into()),
            )
    }

    pub fn row(&self) -> Div {
        self.row_of_height(ROW_HEIGHT)
    }

    pub fn row_described(&self) -> Div {
        self.row_of_height(ROW_DESCRIBED_HEIGHT)
    }

    pub fn row_compact_described(&self) -> Div {
        self.row_of_height(qol_theme::LIST_ENTRY_HEIGHTS[1])
            .h(px(qol_theme::LIST_ENTRY_HEIGHTS[1]))
            .py(px(qol_theme::SPACE_TIGHT))
            .line_height(gpui::relative(1.0))
    }

    pub fn row_tight(&self) -> Div {
        self.row_of_height(ROW_TIGHT_HEIGHT)
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

    pub fn row_selected<E: Styled + ParentElement>(&self, row: E, selected: bool) -> E {
        self.row_state(
            row,
            if selected {
                RowState::Current
            } else {
                RowState::Resting
            },
        )
    }

    pub fn row_selected_tinted<E: Styled + ParentElement>(
        &self,
        row: E,
        selected: bool,
        tone: u32,
    ) -> E {
        self.row_selected_tinted_after(row, selected, tone, 0.0)
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
            .border(px(ROW_BORDER_WIDTH))
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
                .left(px(leading_width - ROW_BORDER_WIDTH))
                .right(px(-ROW_BORDER_WIDTH))
                .top(px(-ROW_BORDER_WIDTH))
                .bottom(px(-ROW_BORDER_WIDTH))
                .border(px(ROW_BORDER_WIDTH))
                .border_color(rgba(colors.selected_edge.packed())),
        )
    }

    pub fn row_state<E: Styled + ParentElement>(&self, row: E, state: RowState) -> E {
        let row = row.relative().border(px(1.0));
        let (wash, edge, bar) = match state {
            RowState::Resting => return row.border_color(rgba(0x00000000)),
            RowState::Disabled => {
                return row.border_color(rgba(0x00000000)).opacity(DISABLED_OPACITY)
            }
            RowState::Hover => (self.washes.fill_hover, None, None),
            RowState::Current => (
                self.washes.wash_selected,
                Some(self.washes.hairline),
                Some(self.palette.accent),
            ),
            RowState::NeedsAttention => {
                (self.washes.wash_attention, None, Some(self.palette.warning))
            }
            RowState::Invalid => (
                self.washes.wash_invalid,
                Some(self.washes.edge_invalid),
                Some(self.palette.danger),
            ),
        };
        let row = row
            .bg(rgba(wash.packed()))
            .border_color(rgba(edge.map(|tone| tone.packed()).unwrap_or(0)));
        match bar {
            None => row,
            Some(tone) => row.overflow_hidden().child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(qol_theme::SPACE_MARK))
                    .bg(rgb(tone)),
            ),
        }
    }

    pub fn label(&self, text: impl Into<SharedString>) -> Div {
        div()
            .truncate()
            .text_size(px(qol_theme::TEXT_BODY))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(self.palette.text_primary))
            .child(text.into())
    }

    pub fn description(&self, text: impl Into<SharedString>) -> Div {
        div()
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(self.palette.text_muted))
            .child(text.into())
    }

    pub fn value(&self, text: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .text_size(px(qol_theme::TEXT_BODY))
            .text_color(rgb(self.palette.text_secondary))
            .child(text.into())
    }

    pub fn mono(&self, text: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .font_family(SharedString::from(qol_theme::font_mono()))
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(self.palette.text_secondary))
            .child(text.into())
    }

    pub fn keycap(&self, text: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .px(px(qol_theme::SPACE_SNUG))
            .py(px(qol_theme::SPACE_STACK))
            .rounded(px(qol_theme::RADIUS_KEYCAP))
            .border(px(1.0))
            .border_color(rgba(self.washes.hairline_strong.packed()))
            .font_family(SharedString::from(qol_theme::font_mono()))
            .text_size(px(qol_theme::TEXT_KEYCAP))
            .text_color(rgb(self.palette.text_muted))
            .child(text.into())
    }

    pub fn status_dot(&self, tone: u32, halo: u32) -> Div {
        div()
            .flex_none()
            .w(px(7.0))
            .h(px(7.0))
            .rounded_full()
            .bg(rgb(tone))
            .shadow(vec![BoxShadow {
                color: rgba(halo).into(),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(3.0),
            }])
    }

    pub fn animated_status_dot(
        &self,
        id: impl Into<gpui::ElementId>,
        tone: u32,
        halo: u32,
        pulsing: bool,
    ) -> gpui::AnyElement {
        let dot = self.status_dot(tone, halo).size(px(LAMP_SIZE));
        if pulsing {
            crate::status_indicator::radiating_dot(dot, id.into(), tone)
        } else {
            dot.into_any_element()
        }
    }

    pub fn count_chip(&self, count: usize, label: impl Into<SharedString>) -> Div {
        self.count_chip_of_height(
            count,
            label,
            qol_theme::HEIGHT_INLINE,
            qol_theme::TEXT_MICRO,
        )
    }

    pub fn count_chip_small(&self, count: usize, label: impl Into<SharedString>) -> Div {
        self.count_chip_of_height(count, label, 22.0, qol_theme::TEXT_NANO)
    }

    fn count_chip_of_height(
        &self,
        count: usize,
        label: impl Into<SharedString>,
        height: f32,
        text_size: f32,
    ) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .h(px(height))
            .px(px(qol_theme::SPACE_INSET))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .border(px(1.0))
            .border_color(rgba(self.washes.hairline.packed()))
            .bg(rgba(self.washes.fill_resting.packed()))
            .text_size(px(text_size))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.text_primary))
                    .child(format!("{count}")),
            )
            .child(
                div()
                    .text_color(rgb(self.palette.text_secondary))
                    .child(label.into()),
            )
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
            .border_t(px(1.0))
            .border_color(rgba(self.washes.hairline.packed()))
            .bg(rgba(self.washes.fill_hover.packed()))
            .text_size(px(qol_theme::TEXT_MICRO))
            .text_color(rgb(self.palette.text_secondary))
    }

    pub fn hint(&self, key: impl Into<SharedString>, label: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .child(self.keycap(key))
            .child(label.into())
    }

    pub fn hint_bar_compact(&self) -> Div {
        self.hint_bar()
            .h(px(qol_theme::HEIGHT_INLINE))
            .bg(rgb(self.palette.surface_canvas))
            .text_size(px(qol_theme::TEXT_NANO))
            .text_color(rgb(self.palette.text_muted))
    }

    pub fn hint_label_first(
        &self,
        label: impl Into<SharedString>,
        key: impl Into<SharedString>,
    ) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .child(label.into())
            .child(
                div()
                    .flex_none()
                    .h(px(qol_theme::SPACE_PAD))
                    .px(px(qol_theme::SPACE_STACK))
                    .flex()
                    .items_center()
                    .rounded(px(qol_theme::RADIUS_TIGHT))
                    .bg(rgba(self.washes.fill_resting.packed()))
                    .font_family(SharedString::from(qol_theme::font_mono()))
                    .text_size(px(qol_theme::TEXT_NANO))
                    .text_color(rgb(self.palette.text_secondary))
                    .child(key.into()),
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
            .text_size(px(qol_theme::TEXT_NANO))
            .font_weight(FontWeight::SEMIBOLD)
            .child(glyph)
    }

    pub fn chip(&self, text: impl Into<SharedString>, tone: u32) -> Div {
        div()
            .flex_none()
            .px(px(qol_theme::SPACE_SNUG))
            .py(px(qol_theme::SPACE_STACK))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .bg(rgba(alpha(tone, 0x33)))
            .text_size(px(qol_theme::TEXT_MICRO))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(tone))
            .child(text.into())
    }

    pub fn vertical_identity_tab(&self, text: impl Into<SharedString>, tone: u32) -> Div {
        div()
            .absolute()
            .left(px(-ROW_BORDER_WIDTH))
            .top(px(-ROW_BORDER_WIDTH))
            .bottom(px(-ROW_BORDER_WIDTH))
            .w(px(crate::vertical_label::WIDTH))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(tone))
            .child(crate::vertical_label::VerticalLabel::new(text))
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

    pub fn count_button(&self, count: usize) -> Div {
        let label = if count > 99 {
            "99+".to_owned()
        } else {
            count.to_string()
        };
        self.button_ghost(label)
            .size(px(qol_theme::HEIGHT_INLINE))
            .p(px(qol_theme::SPACE_STACK))
            .justify_center()
            .text_size(px(qol_theme::TEXT_NANO))
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

    pub fn lamp(&self, tone: u32) -> Div {
        div()
            .flex_none()
            .w(px(LAMP_SIZE))
            .h(px(LAMP_SIZE))
            .rounded_full()
            .bg(rgb(tone))
    }

    pub fn button_primary(&self, text: impl Into<SharedString>) -> Div {
        self.button_base(text)
            .bg(rgb(self.palette.solid_fill))
            .text_color(rgb(self.palette.solid_ink))
            .shadow(raised_shadow(self.palette.text_primary))
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

    pub fn status_pill(&self, text: impl Into<SharedString>, tone: u32) -> Div {
        div()
            .flex_none()
            .h(px(qol_theme::SPACE_PAD))
            .px(px(qol_theme::SPACE_SNUG))
            .flex()
            .items_center()
            .rounded_full()
            .bg(rgba(alpha(tone, 0x33)))
            .text_size(px(qol_theme::TEXT_NANO))
            .text_color(rgb(tone))
            .child(text.into())
    }

    pub fn button_danger(&self, text: impl Into<SharedString>) -> Div {
        self.button_base(text)
            .bg(rgba(alpha(self.palette.danger, 0x29)))
            .text_color(rgb(self.palette.danger))
    }

    fn button_base(&self, text: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .px(px(qol_theme::SPACE_CELL))
            .py(px(qol_theme::SPACE_SNUG))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .text_size(px(qol_theme::TEXT_CAPTION))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text.into())
    }

    pub fn radio(&self, selected: bool) -> Div {
        let outer = div()
            .flex_none()
            .w(px(15.0))
            .h(px(15.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(self.palette.surface_raised))
            .border(px(1.5))
            .border_color(rgb(if selected {
                self.palette.accent
            } else {
                self.palette.border_subtle
            }));
        if !selected {
            return outer;
        }
        outer.child(
            div()
                .w(px(7.0))
                .h(px(7.0))
                .rounded_full()
                .bg(rgb(self.palette.accent)),
        )
    }

    pub fn check(&self, selected: bool) -> Div {
        let base = div()
            .flex_none()
            .w(px(15.0))
            .h(px(15.0))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .flex()
            .items_center()
            .justify_center();
        if selected {
            base.bg(rgb(self.palette.accent))
                .text_color(rgb(self.palette.surface_raised))
                .text_size(px(qol_theme::TEXT_MICRO))
                .font_weight(FontWeight::EXTRA_BOLD)
                .child("\u{2713}")
        } else {
            base.bg(rgb(self.palette.surface_raised))
                .border(px(1.5))
                .border_color(rgb(self.palette.border_subtle))
        }
    }

    pub fn segmented(&self, options: &[SharedString], selected: usize) -> Div {
        let mut group = div()
            .flex_none()
            .flex()
            .flex_row()
            .gap(px(qol_theme::SPACE_STACK))
            .p(px(qol_theme::SPACE_STACK))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .bg(rgb(self.palette.surface_hovered));
        for (index, option) in options.iter().enumerate() {
            let active = index == selected;
            let mut segment = div()
                .px(px(qol_theme::SPACE_CELL))
                .py(px(qol_theme::SPACE_STACK))
                .rounded(px(qol_theme::RADIUS_TIGHT))
                .text_size(px(qol_theme::TEXT_CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .child(option.clone());
            segment = if active {
                segment
                    .bg(rgb(self.palette.accent))
                    .text_color(rgb(self.palette.surface_raised))
                    .shadow(raised_shadow(self.palette.text_primary))
            } else {
                segment.text_color(rgb(self.palette.text_secondary))
            };
            group = group.child(segment);
        }
        group
    }

    pub fn divider(&self) -> Div {
        div()
            .flex_none()
            .w_full()
            .h(px(1.0))
            .bg(rgb(self.palette.border_subtle))
    }
}

fn focus_ring_from(accent: u32, halo: u32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: rgb(accent).into(),
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(0.0),
            spread_radius: px(FOCUS_RING_EDGE),
        },
        BoxShadow {
            color: rgba(halo).into(),
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(0.0),
            spread_radius: px(FOCUS_RING_HALO),
        },
    ]
}

pub fn focus_ring_for(mode: ThemeMode, palette: SystemPalette) -> Vec<BoxShadow> {
    Kit::new(mode, palette).focus_ring()
}

pub fn float_shadow(text_primary: u32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: rgba(alpha(text_primary, 0x14)).into(),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(2.0),
            spread_radius: px(0.0),
        },
        BoxShadow {
            color: rgba(alpha(text_primary, 0x21)).into(),
            offset: point(px(0.0), px(12.0)),
            blur_radius: px(30.0),
            spread_radius: px(0.0),
        },
    ]
}

pub fn raised_shadow(text_primary: u32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: rgba(alpha(text_primary, 0x1a)).into(),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(2.0),
            spread_radius: px(0.0),
        },
        BoxShadow {
            color: rgba(alpha(text_primary, 0x1a)).into(),
            offset: point(px(0.0), px(6.0)),
            blur_radius: px(16.0),
            spread_radius: px(0.0),
        },
    ]
}

pub fn accent_left_edge(radius: f32, width: f32, accent: u32) -> Div {
    div()
        .absolute()
        .inset_0()
        .rounded_l(px(radius))
        .border_l(px(width))
        .border_color(rgb(accent))
}

pub const RAIL_SCRIM_START: f32 = 0.32;
pub const RAIL_SCRIM_END: f32 = 0.5;
pub const RAIL_SCRIM_ALPHA: u8 = 0x66;

pub fn rail_scrim(surface: u32) -> Background {
    linear_gradient(
        90.0,
        linear_color_stop(rgba(alpha(surface, 0x00)), RAIL_SCRIM_START),
        linear_color_stop(rgba(alpha(surface, RAIL_SCRIM_ALPHA)), RAIL_SCRIM_END),
    )
}

pub fn alpha(color: u32, opacity: u8) -> u32 {
    (color << 8) | u32::from(opacity)
}

const TILE_TONES: [u32; 6] = [0x2f7350, 0x3a639b, 0x8a6208, 0x5c626d, 0x2f3238, 0x7a4a8a];

pub fn tile_tone(name: &str) -> u32 {
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
    Kit::new(theme.mode, theme.system)
}

#[cfg(test)]
mod tests {
    use super::{
        focus_ring_for, path_label, rail_scrim, FOCUS_RING_EDGE, FOCUS_RING_HALO, RAIL_SCRIM_ALPHA,
        RAIL_SCRIM_END, RAIL_SCRIM_START,
    };
    use qol_theme::{ThemeMode, DARK_SYSTEM, LIGHT_SYSTEM};

    #[test]
    fn the_focus_ring_keeps_a_solid_inner_edge_inside_a_soft_halo() {
        for (mode, palette) in [
            (ThemeMode::Light, LIGHT_SYSTEM),
            (ThemeMode::Dark, DARK_SYSTEM),
        ] {
            let ring = focus_ring_for(mode, palette);
            assert_eq!(ring.len(), 2);

            let edge = &ring[0];
            assert_eq!(f32::from(edge.spread_radius), FOCUS_RING_EDGE);
            assert_eq!(f32::from(edge.blur_radius), 0.0);
            assert_eq!(edge.color.a, 1.0);

            let halo = &ring[1];
            assert_eq!(f32::from(halo.spread_radius), FOCUS_RING_HALO);
            assert!(halo.color.a > 0.0 && halo.color.a < 1.0);
            assert!(halo.spread_radius > edge.spread_radius);
        }
    }

    #[test]
    fn the_rail_scrim_stays_clear_through_a_third_then_ramps_to_full_alpha() {
        let rendered = format!("{:?}", rail_scrim(0x000000));
        assert!(rendered.contains("LinearGradient(90"));
        assert!(rendered.contains(&format!("percentage: {RAIL_SCRIM_START}")));
        assert!(rendered.contains(&format!("percentage: {RAIL_SCRIM_END}")));
        assert!(rendered.contains("a: 0.0"));
        assert!(rendered.contains(&format!("a: {}", f32::from(RAIL_SCRIM_ALPHA) / 255.0)));
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
