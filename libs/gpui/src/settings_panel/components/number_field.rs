use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba};
use qol_theme::TextStyle;

use crate::theme::SettingsPanelPalette;

use super::{ground_bg, ground_text, RowGround};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::settings_panel) enum SliderStyle {
    Compact,
    Wide,
}

impl SliderStyle {
    pub(in crate::settings_panel) fn from_variant(variant: Option<&str>) -> Option<Self> {
        match variant {
            Some("slider") => Some(Self::Compact),
            Some("wide_slider") => Some(Self::Wide),
            _ => None,
        }
    }

    fn track_width(self) -> f32 {
        match self {
            Self::Compact => 72.0,
            Self::Wide => 240.0,
        }
    }

    fn track_height(self) -> f32 {
        match self {
            Self::Compact => 4.0,
            Self::Wide => 6.0,
        }
    }
}

const WIDE_THUMB: f32 = 14.0;

fn thumb_left(centre: f32) -> f32 {
    centre - WIDE_THUMB / 2.0
}

pub(in crate::settings_panel) fn number_field(
    display: String,
    unit: Option<&'static str>,
    track: Option<(f32, SliderStyle)>,
    interact: impl FnOnce(gpui::Div) -> gpui::Div,
    row: RowGround,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    let ground = row.rest(palette);
    let hover = row.hover(palette);
    let (text, text_hover) = match row {
        RowGround::Pane => (ground.soft, None),
        RowGround::Band => (ground.ink, hover.map(|hover| hover.ink)),
    };
    let mut cell = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET));
    if let Some((fraction, style)) = track {
        let width = style.track_width();
        let fill = fraction * width;
        let bar = ground_bg(
            div()
                .relative()
                .w(px(width))
                .h(px(style.track_height()))
                .rounded_full()
                .overflow_hidden(),
            rgba(ground.well.packed()),
            hover.map(|hover| rgba(hover.well.packed())),
        )
        .child(ground_bg(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(px(fill))
                .rounded_full(),
            rgb(ground.mark),
            hover.map(|hover| rgb(hover.mark)),
        ));
        cell = cell.child(interact(match style {
            SliderStyle::Compact => bar,
            SliderStyle::Wide => div()
                .relative()
                .flex()
                .items_center()
                .w(px(width))
                .h(px(WIDE_THUMB))
                .child(bar)
                .child(ground_bg(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(thumb_left(fill)))
                        .size(px(WIDE_THUMB))
                        .rounded_full(),
                    rgb(ground.mark),
                    hover.map(|hover| rgb(hover.mark)),
                )),
        }));
    }
    let chip = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET))
        .h(px(qol_theme::HEIGHT_INLINE))
        .px(px(qol_theme::SPACE_INSET))
        .rounded(px(qol_theme::RADIUS_CONTROL))
        .border(px(1.0))
        .border_color(rgba(ground.edge.packed()))
        .child(
            ground_text(div().text(TextStyle::Value), rgb(text), text_hover.map(rgb))
                .child(display),
        )
        .children(unit.map(|unit| {
            ground_text(
                div().text(TextStyle::Detail),
                rgb(ground.faint),
                hover.map(|hover| rgb(hover.faint)),
            )
            .child(unit)
        }));
    cell.child(ground_bg(
        chip,
        rgba(ground.well.packed()),
        hover.map(|hover| rgba(hover.well.packed())),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fill_edge_meets_the_thumb_centre_at_every_fraction() {
        for width in [
            SliderStyle::Compact.track_width(),
            SliderStyle::Wide.track_width(),
        ] {
            for fraction in [0.0, 0.1, 0.5, 0.9, 1.0] {
                let fill_edge = fraction * width;
                let thumb_centre = thumb_left(fill_edge) + WIDE_THUMB / 2.0;
                assert!(
                    (fill_edge - thumb_centre).abs() < f32::EPSILON,
                    "width {width} fraction {fraction}: fill {fill_edge} thumb {thumb_centre}"
                );
            }
        }
    }

    #[test]
    fn the_wide_slider_is_opt_in_by_variant() {
        assert_eq!(
            SliderStyle::from_variant(Some("slider")),
            Some(SliderStyle::Compact)
        );
        assert_eq!(
            SliderStyle::from_variant(Some("wide_slider")),
            Some(SliderStyle::Wide)
        );
        assert_eq!(SliderStyle::from_variant(None), None);
        assert_eq!(SliderStyle::from_variant(Some("danger")), None);
        assert!(SliderStyle::Wide.track_width() > SliderStyle::Compact.track_width());
    }
}
