use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba};
use qol_theme::TextStyle;

use crate::kit::Kit;
use crate::theme::Ground;

use super::{ground_bg, ground_text, RowGround};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::settings_panel) enum SliderStyle {
    Compact,
    Wide,
    Level,
}

impl SliderStyle {
    pub(in crate::settings_panel) fn from_variant(variant: Option<&str>) -> Option<Self> {
        match variant {
            Some("slider") => Some(Self::Compact),
            Some("wide_slider") => Some(Self::Wide),
            Some("level_slider") => Some(Self::Level),
            _ => None,
        }
    }

    fn track_width(self) -> f32 {
        match self {
            Self::Compact => 72.0,
            Self::Wide | Self::Level => 240.0,
        }
    }

    fn track_height(self) -> f32 {
        match self {
            Self::Compact => 4.0,
            Self::Wide => 6.0,
            Self::Level => LEVEL_SEGMENT_HEIGHT,
        }
    }
}

const WIDE_THUMB: f32 = 14.0;
const LEVEL_SEGMENTS: usize = 16;
const LEVEL_SEGMENT_HEIGHT: f32 = 10.0;

fn thumb_left(centre: f32) -> f32 {
    centre - WIDE_THUMB / 2.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LevelSegment {
    Lit,
    Unlit,
}

fn segments_reached(fraction: f32) -> usize {
    ((fraction.clamp(0.0, 1.0) * LEVEL_SEGMENTS as f32).round() as usize).min(LEVEL_SEGMENTS)
}

fn segments_lit(level: f32, fraction: f32) -> usize {
    let lit = (level.clamp(0.0, 1.0) * fraction.clamp(0.0, 1.0) * LEVEL_SEGMENTS as f32).round();
    (lit as usize).min(segments_reached(fraction))
}

fn level_segment(index: usize, level: f32, fraction: f32) -> LevelSegment {
    if index < segments_lit(level, fraction) {
        LevelSegment::Lit
    } else {
        LevelSegment::Unlit
    }
}

fn level_segment_width(track: f32) -> f32 {
    let gaps = (LEVEL_SEGMENTS - 1) as f32 * qol_theme::SPACE_TIGHT;
    (track - gaps) / LEVEL_SEGMENTS as f32
}

fn slider_thumb(fill: f32, ground: Ground, hover: Option<Ground>) -> gpui::Div {
    ground_bg(
        div()
            .absolute()
            .top_0()
            .left(px(thumb_left(fill)))
            .size(px(WIDE_THUMB))
            .rounded_full(),
        rgb(ground.mark),
        hover.map(|hover| rgb(hover.mark)),
    )
}

fn level_segments(
    fraction: f32,
    level: f32,
    width: f32,
    ground: Ground,
    hover: Option<Ground>,
) -> gpui::Div {
    let segment_width = level_segment_width(width);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_TIGHT))
        .w(px(width))
        .children((0..LEVEL_SEGMENTS).map(|index| {
            let segment = div()
                .flex_none()
                .w(px(segment_width))
                .h(px(LEVEL_SEGMENT_HEIGHT))
                .rounded(px(qol_theme::RADIUS_THUMB));
            match level_segment(index, level, fraction) {
                LevelSegment::Lit => ground_bg(
                    segment,
                    rgb(ground.mark),
                    hover.map(|hover| rgb(hover.mark)),
                ),
                LevelSegment::Unlit => ground_bg(
                    segment,
                    rgba(ground.well.packed()),
                    hover.map(|hover| rgba(hover.well.packed())),
                ),
            }
        }))
}

fn slider_bar(style: SliderStyle, fill: f32, ground: Ground, hover: Option<Ground>) -> gpui::Div {
    ground_bg(
        div()
            .relative()
            .w(px(style.track_width()))
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
    ))
}

pub(in crate::settings_panel) fn number_field(
    display: String,
    unit: Option<&'static str>,
    track: Option<(f32, SliderStyle)>,
    level: f32,
    interact: impl FnOnce(gpui::Div) -> gpui::Div,
    row: RowGround,
    kit: Kit,
) -> gpui::Div {
    let ground = row.rest(kit);
    let hover = row.hover(kit);
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
        let with_thumb = |track: gpui::Div| {
            div()
                .relative()
                .flex()
                .items_center()
                .w(px(width))
                .h(px(WIDE_THUMB))
                .child(track)
                .child(slider_thumb(fill, ground, hover))
        };
        cell = cell.child(interact(match style {
            SliderStyle::Compact => slider_bar(style, fill, ground, hover),
            SliderStyle::Wide => with_thumb(slider_bar(style, fill, ground, hover)),
            SliderStyle::Level => with_thumb(level_segments(fraction, level, width, ground, hover)),
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
        .border(px(qol_theme::LINE))
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
            SliderStyle::Level.track_width(),
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
        assert_eq!(
            SliderStyle::from_variant(Some("level_slider")),
            Some(SliderStyle::Level)
        );
        assert_eq!(SliderStyle::from_variant(None), None);
        assert_eq!(SliderStyle::from_variant(Some("danger")), None);
        assert!(SliderStyle::Wide.track_width() > SliderStyle::Compact.track_width());
    }

    const STEPS: [f32; 11] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];

    #[test]
    fn the_lit_segments_never_pass_the_thumb() {
        for fraction in STEPS {
            for level in STEPS.into_iter().chain([1.5, -0.5, f32::NAN]) {
                let lit = segments_lit(level, fraction);
                assert!(
                    lit <= segments_reached(fraction),
                    "fraction {fraction} level {level}: {lit} lit"
                );
                for index in 0..LEVEL_SEGMENTS {
                    let lit_here = level_segment(index, level, fraction) == LevelSegment::Lit;
                    assert_eq!(lit_here, index < lit, "fraction {fraction} level {level}");
                }
            }
        }
        assert_eq!(segments_lit(1.0, 1.0), LEVEL_SEGMENTS);
        assert_eq!(segments_lit(0.42, 0.7), 5);
        assert_eq!(segments_lit(0.0, 1.0), 0);
    }

    #[test]
    fn silence_leaves_every_segment_unlit_wherever_the_thumb_sits() {
        for fraction in STEPS {
            for index in 0..LEVEL_SEGMENTS {
                assert_eq!(level_segment(index, 0.0, fraction), LevelSegment::Unlit);
            }
        }
    }

    #[test]
    fn the_segments_and_their_gaps_fill_the_wide_track() {
        let width = SliderStyle::Level.track_width();
        let segment = level_segment_width(width);
        let total =
            segment * LEVEL_SEGMENTS as f32 + qol_theme::SPACE_TIGHT * (LEVEL_SEGMENTS - 1) as f32;
        assert!((total - width).abs() < 0.001, "{total}");
        assert_eq!(width, SliderStyle::Wide.track_width());
    }
}
