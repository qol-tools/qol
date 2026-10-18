use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{canvas, div, point, px, rgb, rgba, PathBuilder};
use qol_theme::TextStyle;

use crate::kit::Kit;
use crate::theme::Ground;

use super::{ground_bg, ground_text, RowGround, SETTINGS_ROW_GROUP};

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

    pub(in crate::settings_panel) fn track_width(self) -> f32 {
        match self {
            Self::Compact => 128.0,
            Self::Wide | Self::Level => 240.0,
        }
    }

    fn track_height(self) -> f32 {
        match self {
            Self::Compact => qol_theme::HEIGHT_INLINE,
            Self::Wide => 6.0,
            Self::Level => LEVEL_SEGMENT_HEIGHT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::settings_panel) struct NumberCue {
    pub(in crate::settings_panel) level: f32,
    pub(in crate::settings_panel) editing: bool,
    pub(in crate::settings_panel) dragging: bool,
    pub(in crate::settings_panel) limit: Option<String>,
}

const WIDE_THUMB: f32 = 16.0;
const THUMB_RING: f32 = 3.0;
const THUMB_HALO: f32 = 6.0;
const CARET_WIDTH: f32 = 2.0;
const CARET_HEIGHT: f32 = 18.0;
const NUDGE_CHEVRON_WIDTH: f32 = 6.0;
const NUDGE_CHEVRON_HEIGHT: f32 = 10.0;
const NUDGE_CHEVRON_STROKE: f32 = 1.5;
const TICK_HEIGHT: f32 = 4.0;
const TICKS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
const LEVEL_SEGMENTS: usize = 16;
const LEVEL_SEGMENT_HEIGHT: f32 = 10.0;
const LEVEL_LOUD_FROM: f32 = 0.7;
const LEVEL_PEAK_FROM: f32 = 0.9;

fn thumb_left(centre: f32) -> f32 {
    centre - WIDE_THUMB / 2.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LevelSegment {
    Quiet,
    Loud,
    Peak,
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
    if index >= segments_lit(level, fraction) {
        return LevelSegment::Unlit;
    }
    let reach = (index + 1) as f32 / LEVEL_SEGMENTS as f32;
    if reach > LEVEL_PEAK_FROM {
        LevelSegment::Peak
    } else if reach > LEVEL_LOUD_FROM {
        LevelSegment::Loud
    } else {
        LevelSegment::Quiet
    }
}

fn tone(color: u32, alpha: f32) -> gpui::Rgba {
    let mut tone = rgb(color);
    tone.a = alpha;
    tone
}

fn level_tone(color: u32) -> gpui::Rgba {
    tone(color, qol_theme::OPACITY_REST)
}

fn level_segment_width(track: f32) -> f32 {
    let gaps = (LEVEL_SEGMENTS - 1) as f32 * qol_theme::SPACE_TIGHT;
    (track - gaps) / LEVEL_SEGMENTS as f32
}

fn slider_thumb(fill: f32, dragging: bool, ground: Ground, hover: Option<Ground>) -> gpui::Div {
    let halo = div()
        .absolute()
        .top(px(-THUMB_HALO))
        .left(px(-THUMB_HALO))
        .size(px(WIDE_THUMB + 2.0 * THUMB_HALO))
        .rounded_full();
    let resting_halo = rgba(ground.halo.packed());
    let halo = if dragging {
        ground_bg(
            halo,
            resting_halo,
            hover.map(|hover| rgba(hover.halo.packed())),
        )
    } else {
        let lifted_halo = rgba(hover.unwrap_or(ground).halo.packed());
        halo.bg(resting_halo)
            .opacity(0.0)
            .group_hover(SETTINGS_ROW_GROUP, move |style| {
                style.bg(lifted_halo).opacity(1.0)
            })
    };
    let face = ground_bg(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full(),
        rgb(ground.mark),
        hover.map(|hover| rgb(hover.mark)),
    );
    let face = if dragging {
        face
    } else {
        face.child(ground_bg(
            div().size(px(WIDE_THUMB - 2.0 * THUMB_RING)).rounded_full(),
            rgb(ground.bg),
            hover.map(|hover| rgb(hover.bg)),
        ))
    };
    div()
        .absolute()
        .top_0()
        .left(px(thumb_left(fill)))
        .size(px(WIDE_THUMB))
        .child(halo)
        .child(face)
}

fn quarter_ticks(
    width: f32,
    ground: Ground,
    hover: Option<Ground>,
) -> impl Iterator<Item = gpui::Div> {
    TICKS.into_iter().map(move |at| {
        ground_bg(
            div()
                .absolute()
                .top(px(WIDE_THUMB))
                .left(px(at * width))
                .w(px(qol_theme::LINE))
                .h(px(TICK_HEIGHT)),
            rgb(ground.faint),
            hover.map(|hover| rgb(hover.faint)),
        )
    })
}

fn level_segments(
    fraction: f32,
    level: f32,
    width: f32,
    ground: Ground,
    hover: Option<Ground>,
    kit: Kit,
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
                LevelSegment::Quiet => segment.bg(level_tone(kit.palette.success)),
                LevelSegment::Loud => segment.bg(level_tone(kit.palette.warning)),
                LevelSegment::Peak => segment.bg(level_tone(kit.palette.danger)),
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

fn nudge_chevron(points_right: bool, ink: u32, shown: bool) -> gpui::Div {
    let mark = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let inset = px(qol_theme::LINE);
            let (tail, tip) = if points_right {
                (bounds.left() + inset, bounds.right() - inset)
            } else {
                (bounds.right() - inset, bounds.left() + inset)
            };
            let mut path = PathBuilder::stroke(px(NUDGE_CHEVRON_STROKE));
            path.move_to(point(tail, bounds.top() + inset));
            path.line_to(point(tip, bounds.center().y));
            path.line_to(point(tail, bounds.bottom() - inset));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(ink));
            }
        },
    )
    .w(px(NUDGE_CHEVRON_WIDTH))
    .h(px(NUDGE_CHEVRON_HEIGHT));
    let end = div()
        .absolute()
        .top_0()
        .h_full()
        .flex()
        .items_center()
        .child(mark);
    let end = if points_right {
        end.right(px(qol_theme::SPACE_INSET))
    } else {
        end.left(px(qol_theme::SPACE_INSET))
    };
    if shown {
        end
    } else {
        end.opacity(0.0)
            .group_hover(SETTINGS_ROW_GROUP, |style| style.opacity(1.0))
    }
}

fn number_value(
    display: String,
    unit: Option<&'static str>,
    cue: &NumberCue,
    ground: Ground,
    hover: Option<Ground>,
) -> gpui::Div {
    let digits = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_STACK))
        .child(
            ground_text(
                div().text(TextStyle::Value),
                rgb(ground.ink),
                hover.map(|hover| rgb(hover.ink)),
            )
            .child(display),
        )
        .when(cue.editing, |digits| {
            digits.child(ground_bg(
                div().flex_none().w(px(CARET_WIDTH)).h(px(CARET_HEIGHT)),
                rgb(ground.ink),
                hover.map(|hover| rgb(hover.ink)),
            ))
        });
    let faint = |style: TextStyle| {
        ground_text(
            div().text(style),
            rgb(ground.faint),
            hover.map(|hover| rgb(hover.faint)),
        )
    };
    let after = match (&cue.limit, unit) {
        (Some(limit), _) => Some(faint(TextStyle::Key).child(limit.clone())),
        (None, Some(unit)) => Some(faint(TextStyle::Detail).child(unit)),
        (None, None) => None,
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET))
        .child(digits)
        .children(after)
}

fn scrub_field(
    value: gpui::Div,
    fill: f32,
    cue: &NumberCue,
    ground: Ground,
    hover: Option<Ground>,
    kit: Kit,
) -> gpui::Div {
    let strength = if cue.dragging {
        qol_theme::OPACITY_FILL_HELD
    } else {
        qol_theme::OPACITY_FILL
    };
    let chevron = hover.unwrap_or(ground).soft;
    ground_bg(
        div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .w(px(SliderStyle::Compact.track_width()))
            .h(px(SliderStyle::Compact.track_height()))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .overflow_hidden(),
        rgba(ground.well.packed()),
        hover.map(|hover| rgba(hover.well.packed())),
    )
    .child(ground_bg(
        div().absolute().left_0().top_0().h_full().w(px(fill)),
        tone(ground.mark, strength),
        hover.map(|hover| tone(hover.mark, strength)),
    ))
    .child(nudge_chevron(false, chevron, cue.dragging))
    .child(nudge_chevron(true, chevron, cue.dragging))
    .child(value)
    .when(cue.editing, |field| field.shadow(kit.focus_ring(ground)))
}

pub(in crate::settings_panel) fn number_field(
    display: String,
    unit: Option<&'static str>,
    track: Option<(f32, SliderStyle)>,
    cue: NumberCue,
    interact: impl FnOnce(gpui::Div) -> gpui::Div,
    row: RowGround,
    kit: Kit,
) -> gpui::Div {
    let ground = row.rest(kit);
    let hover = row.hover(kit);
    let value = number_value(display, unit, &cue, ground, hover);
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
                .child(slider_thumb(fill, cue.dragging, ground, hover))
        };
        let bar = match style {
            SliderStyle::Compact => {
                return interact(scrub_field(value, fill, &cue, ground, hover, kit));
            }
            SliderStyle::Wide => with_thumb(slider_bar(style, fill, ground, hover))
                .children(quarter_ticks(width, ground, hover)),
            SliderStyle::Level => with_thumb(level_segments(
                fraction, cue.level, width, ground, hover, kit,
            )),
        };
        cell = cell.child(interact(bar));
    }
    let chip = div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(qol_theme::HEIGHT_INLINE))
        .px(px(qol_theme::SPACE_INSET))
        .rounded(px(qol_theme::RADIUS_CONTROL))
        .child(value)
        .when(cue.editing, |chip| chip.shadow(kit.focus_ring(ground)));
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
                    let lit_here = level_segment(index, level, fraction) != LevelSegment::Unlit;
                    assert_eq!(lit_here, index < lit, "fraction {fraction} level {level}");
                }
            }
        }
        assert_eq!(segments_lit(1.0, 1.0), LEVEL_SEGMENTS);
        assert_eq!(segments_lit(0.42, 0.7), 5);
        assert_eq!(segments_lit(0.0, 1.0), 0);
    }

    #[test]
    fn the_lit_part_turns_loud_past_seventy_and_peaks_past_ninety_percent() {
        let tones: Vec<LevelSegment> = (0..LEVEL_SEGMENTS)
            .map(|index| level_segment(index, 1.0, 1.0))
            .collect();
        assert!(tones[..11].iter().all(|tone| *tone == LevelSegment::Quiet));
        assert!(tones[11..14].iter().all(|tone| *tone == LevelSegment::Loud));
        assert!(tones[14..].iter().all(|tone| *tone == LevelSegment::Peak));
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
