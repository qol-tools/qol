use gpui::prelude::*;
use gpui::{div, point, px, rgb, rgba, BoxShadow, Div, FontWeight, SharedString};
use qol_theme::SystemPalette;

pub const FLOAT_SHADOW_OFFSET: f32 = 2.0;
pub const FLOAT_SHADOW_ALPHA: u8 = 0x1a;

pub const HEADER_HEIGHT: f32 = 42.0;
pub const SECTION_HEIGHT: f32 = 30.0;
pub const ROW_HEIGHT: f32 = 44.0;
pub const ROW_DESCRIBED_HEIGHT: f32 = 58.0;
pub const ROW_TIGHT_HEIGHT: f32 = 32.0;
pub const GUTTER: f32 = 16.0;
pub const LAMP_SIZE: f32 = 10.0;

#[derive(Clone, Copy)]
pub struct Kit {
    pub palette: SystemPalette,
}

impl Kit {
    pub fn new(palette: SystemPalette) -> Self {
        Self { palette }
    }

    pub fn panel(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .rounded_none()
            .shadow(float_shadow(self.palette.text_primary))
            .bg(rgb(self.palette.surface_elevated))
    }

    pub fn header(&self, title: impl Into<SharedString>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
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
            .gap_1p5()
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

    pub fn row_tight(&self) -> Div {
        self.row_of_height(ROW_TIGHT_HEIGHT)
    }

    fn row_of_height(&self, height: f32) -> Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_3()
            .w_full()
            .min_h(px(height))
            .px(px(GUTTER))
            .rounded_none()
    }

    pub fn row_selected(&self, row: Div, selected: bool) -> Div {
        let row = row.relative();
        if !selected {
            return row;
        }
        row.bg(rgb(self.palette.accent_fill)).child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(3.0))
                .bg(rgb(self.palette.accent)),
        )
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
            .px_1p5()
            .py_0p5()
            .rounded_none()
            .font_family(SharedString::from(qol_theme::font_mono()))
            .text_size(px(qol_theme::TEXT_MICRO))
            .bg(rgb(self.palette.surface_raised))
            .shadow(vec![BoxShadow {
                color: rgb(self.palette.border_subtle).into(),
                offset: point(px(0.0), px(1.5)),
                blur_radius: px(0.0),
                spread_radius: px(0.0),
            }])
            .text_color(rgb(self.palette.text_secondary))
            .child(text.into())
    }

    pub fn chip(&self, text: impl Into<SharedString>, tone: u32) -> Div {
        div()
            .flex_none()
            .px_1p5()
            .py_0p5()
            .rounded_none()
            .bg(rgba(alpha(tone, 0x33)))
            .text_size(px(qol_theme::TEXT_MICRO))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(tone))
            .child(text.into())
    }

    pub fn lamp(&self, tone: u32) -> Div {
        div()
            .flex_none()
            .w(px(LAMP_SIZE))
            .h(px(LAMP_SIZE))
            .rounded_none()
            .bg(rgb(tone))
    }

    pub fn button_primary(&self, text: impl Into<SharedString>) -> Div {
        self.button_base(text)
            .bg(rgb(self.palette.accent))
            .text_color(rgb(self.palette.surface_raised))
            .shadow(raised_shadow(self.palette.text_primary))
    }

    pub fn button_ghost(&self, text: impl Into<SharedString>) -> Div {
        self.button_base(text)
            .bg(rgb(self.palette.surface_raised))
            .text_color(rgb(self.palette.text_secondary))
            .shadow(raised_shadow(self.palette.text_primary))
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
            .px_3()
            .py_1p5()
            .rounded_none()
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
            .rounded_none()
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
            .gap_0p5()
            .p_0p5()
            .rounded_none()
            .bg(rgb(self.palette.surface_hovered));
        for (index, option) in options.iter().enumerate() {
            let active = index == selected;
            let mut segment = div()
                .px_3()
                .py_0p5()
                .rounded_none()
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

pub fn alpha(color: u32, opacity: u8) -> u32 {
    (color << 8) | u32::from(opacity)
}

pub fn kit() -> Kit {
    Kit::new(qol_theme::runtime_theme().system)
}
