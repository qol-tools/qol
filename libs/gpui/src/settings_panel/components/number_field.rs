use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, point, px, rgb, rgba, AnimationExt, BoxShadow, SharedString};
use qol_theme::{Curve, Motion, TextStyle};
use std::time::Duration;

use crate::deck::with_alpha;
use crate::kit::Kit;
use crate::theme::Ground;

use super::{ground_bg, ground_text, settings_selected_text, RowGround, SETTINGS_ROW_GROUP};

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

    pub(in crate::settings_panel) fn thumb_reach(self) -> f32 {
        match self {
            Self::Compact => 0.0,
            Self::Wide | Self::Level => WIDE_THUMB / 2.0,
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

#[derive(Debug, PartialEq)]
pub(in crate::settings_panel) struct NumberCue {
    pub(in crate::settings_panel) level: f32,
    pub(in crate::settings_panel) editing: bool,
    pub(in crate::settings_panel) dragging: bool,
    pub(in crate::settings_panel) limit: Option<String>,
    pub(in crate::settings_panel) marked: bool,
    pub(in crate::settings_panel) pulse: Option<usize>,
    pub(in crate::settings_panel) settle: Option<(f32, usize)>,
}

impl NumberCue {
    pub(in crate::settings_panel) const SETTLE: Motion = Motion {
        duration: Duration::from_millis(200),
        curve: Curve::Settle,
    };
    pub(in crate::settings_panel) const PULSE: Duration = Motion::TRAVEL.duration;
}

const WIDE_THUMB: f32 = 16.0;
const THUMB_RING: f32 = 3.0;
const THUMB_HALO: f32 = 6.0;
const CARET_WIDTH: f32 = 2.0;
const CARET_HEIGHT: f32 = 18.0;
const TICK_HEIGHT: f32 = 4.0;
const TICKS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
const LEVEL_SEGMENTS: usize = 16;
const LEVEL_SEGMENT_HEIGHT: f32 = 10.0;
const LEVEL_LOUD_FROM: f32 = 0.7;
const LEVEL_PEAK_FROM: f32 = 0.9;
const MARKER_THUMB: f32 = 12.0;
const MARKER_GLOW: f32 = 4.0;
const MARKER_GLOW_ALPHA: f32 = 0.55;

fn thumb_left(centre: f32) -> f32 {
    centre - WIDE_THUMB / 2.0
}

fn marker_pin_left(fraction: f32) -> f32 {
    tick_left(fraction, SliderStyle::Compact.track_width())
}

fn marker_pin_inset(left: f32) -> f32 {
    let width = SliderStyle::Compact.track_width();
    let radius = qol_theme::RADIUS_CONTROL;
    let centre = left + qol_theme::LINE / 2.0;
    let edge = centre.min(width - centre);
    if edge >= radius {
        return 0.0;
    }
    radius - (radius * radius - (radius - edge) * (radius - edge)).sqrt()
}

fn marker_thumb_left(fraction: f32) -> f32 {
    let width = SliderStyle::Compact.track_width();
    let half = MARKER_THUMB / 2.0;
    (fraction * width).clamp(half, width - half) - half
}

fn marker_thumb_top() -> f32 {
    SliderStyle::Compact.track_height() - MARKER_THUMB / 2.0
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

fn segment_fill(segment: LevelSegment, ground: Ground) -> gpui::Rgba {
    match segment {
        LevelSegment::Quiet => rgb(ground.level.quiet),
        LevelSegment::Loud => rgb(ground.level.loud),
        LevelSegment::Peak => rgb(ground.level.peak),
        LevelSegment::Unlit => rgba(ground.well.packed()),
    }
}

fn tick_left(at: f32, width: f32) -> f32 {
    at * (width - qol_theme::LINE)
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
                .left(px(tick_left(at, width)))
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
) -> gpui::Div {
    let segment_width = level_segment_width(width);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_TIGHT))
        .w(px(width))
        .children((0..LEVEL_SEGMENTS).map(|index| {
            let segment = level_segment(index, level, fraction);
            ground_bg(
                div()
                    .flex_none()
                    .w(px(segment_width))
                    .h(px(LEVEL_SEGMENT_HEIGHT))
                    .rounded(px(qol_theme::RADIUS_THUMB)),
                segment_fill(segment, ground),
                hover.map(|hover| segment_fill(segment, hover)),
            )
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

pub(in crate::settings_panel) struct NumberText {
    pub(in crate::settings_panel) shown: String,
    pub(in crate::settings_panel) widest: String,
    pub(in crate::settings_panel) selected: bool,
}

fn number_value(
    text: NumberText,
    unit: Option<SharedString>,
    cue: &NumberCue,
    ground: Ground,
    hover: Option<Ground>,
    kit: Kit,
) -> gpui::Div {
    let shown = div().absolute().top_0().right_0().text(TextStyle::Value);
    let shown = if text.selected {
        settings_selected_text(shown, kit)
    } else {
        ground_text(shown, rgb(ground.ink), hover.map(|hover| rgb(hover.ink)))
    };
    let caret = cue.editing && !text.selected;
    let digits = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_STACK))
        .child(
            div()
                .relative()
                .child(ground_text(div().text(TextStyle::Value), rgba(0), None).child(text.widest))
                .child(shown.child(text.shown)),
        )
        .when(caret, |digits| {
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

fn settling(
    piece: gpui::Div,
    name: &'static str,
    to: f32,
    settle: Option<(f32, usize)>,
    place: impl Fn(gpui::Div, f32) -> gpui::Div + 'static,
) -> gpui::AnyElement {
    let Some((from, step)) = settle else {
        return place(piece, to).into_any_element();
    };
    piece
        .with_animation(
            (name, step),
            crate::motion::animation(NumberCue::SETTLE),
            move |piece, delta| place(piece, from + (to - from) * delta),
        )
        .into_any_element()
}

fn marker_glow(ground: Ground) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: with_alpha(ground.ink, MARKER_GLOW_ALPHA).into(),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(MARKER_GLOW),
        spread_radius: px(0.0),
    }]
}

fn glowing(face: gpui::Div, ground: Ground) -> gpui::Div {
    face.shadow(marker_glow(ground))
}

fn marker_thumb(
    fraction: f32,
    cue: &NumberCue,
    ground: Ground,
    hover: Option<Ground>,
) -> gpui::AnyElement {
    let face = ground_bg(
        div().size_full().rounded_full(),
        rgb(ground.ink),
        hover.map(|hover| rgb(hover.ink)),
    );
    let face = match cue.pulse {
        _ if cue.dragging => glowing(face, ground).into_any_element(),
        Some(step) => face
            .with_animation(
                ("settings-number-pulse", step),
                crate::motion::animation(Motion::TRAVEL),
                move |face, delta| {
                    if delta < 1.0 {
                        glowing(face, ground)
                    } else {
                        face
                    }
                },
            )
            .into_any_element(),
        None => face.into_any_element(),
    };
    settling(
        div()
            .absolute()
            .top(px(marker_thumb_top()))
            .size(px(MARKER_THUMB))
            .child(face),
        "settings-number-thumb",
        fraction,
        cue.settle,
        |thumb, at| thumb.left(px(marker_thumb_left(at))),
    )
}

fn scrub_field(
    value: gpui::Div,
    fraction: f32,
    cue: &NumberCue,
    ground: Ground,
    hover: Option<Ground>,
    kit: Kit,
) -> gpui::Div {
    let width = SliderStyle::Compact.track_width();
    let strength = if cue.dragging {
        qol_theme::OPACITY_FILL_HELD
    } else {
        qol_theme::OPACITY_FILL
    };
    let fill = settling(
        ground_bg(
            div().absolute().left_0().top_0().h_full(),
            with_alpha(ground.mark, strength),
            hover.map(|hover| with_alpha(hover.mark, strength)),
        ),
        "settings-number-fill",
        fraction,
        cue.settle,
        move |fill, at| fill.w(px(at * width)),
    );
    let pin = cue.marked.then(|| {
        settling(
            ground_bg(
                div().absolute().w(px(qol_theme::LINE)),
                rgb(ground.ink),
                hover.map(|hover| rgb(hover.ink)),
            ),
            "settings-number-pin",
            fraction,
            cue.settle,
            |pin, at| {
                let left = marker_pin_left(at);
                let inset = marker_pin_inset(left);
                pin.left(px(left)).top(px(inset)).bottom(px(inset))
            },
        )
    });
    let field = ground_bg(
        div()
            .absolute()
            .inset_0()
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .overflow_hidden(),
        rgba(ground.well.packed()),
        hover.map(|hover| rgba(hover.well.packed())),
    )
    .child(fill)
    .children(pin)
    .when(cue.editing, |field| field.shadow(kit.focus_ring(ground)));
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(SliderStyle::Compact.track_height()))
        .child(field)
        .children(
            cue.marked
                .then(|| marker_thumb(fraction, cue, ground, hover)),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .child(value),
        )
}

pub(in crate::settings_panel) fn number_field(
    mut text: NumberText,
    unit: Option<SharedString>,
    track: Option<(f32, SliderStyle)>,
    cue: NumberCue,
    interact: impl FnOnce(gpui::Div) -> gpui::Div,
    row: RowGround,
    kit: Kit,
) -> gpui::Div {
    let ground = row.rest(kit);
    let hover = row.hover(kit);
    let compact = matches!(track, Some((_, SliderStyle::Compact)));
    if compact {
        text.widest = text.shown.clone();
    }
    let value = number_value(text, unit, &cue, ground, hover, kit)
        .when(compact, |value| value.gap(px(qol_theme::SPACE_TIGHT)));
    let gap = track.map_or(qol_theme::SPACE_INSET, |_| qol_theme::SPACE_CELL);
    let mut cell = div().flex().flex_row().items_center().gap(px(gap));
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
                return interact(scrub_field(value, fraction, &cue, ground, hover, kit));
            }
            SliderStyle::Wide => with_thumb(slider_bar(style, fill, ground, hover))
                .children(quarter_ticks(width, ground, hover)),
            SliderStyle::Level => {
                with_thumb(level_segments(fraction, cue.level, width, ground, hover))
            }
        };
        cell = cell.child(div().px(px(style.thumb_reach())).child(interact(bar)));
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
    fn a_thumbed_slider_reserves_room_for_the_thumb_past_both_track_ends() {
        assert_eq!(SliderStyle::Compact.thumb_reach(), 0.0);
        for style in [SliderStyle::Wide, SliderStyle::Level] {
            let reach = style.thumb_reach();
            assert_eq!(thumb_left(0.0), -reach);
            assert_eq!(
                thumb_left(style.track_width()) + WIDE_THUMB,
                style.track_width() + reach
            );
        }
    }

    #[test]
    fn the_compact_marker_pin_rides_the_boundary_and_the_thumb_stays_inside_the_field() {
        let width = SliderStyle::Compact.track_width();
        assert_eq!(marker_pin_left(0.0), 0.0);
        assert_eq!(marker_pin_left(1.0), width - qol_theme::LINE);
        assert_eq!(marker_pin_inset(marker_pin_left(0.5)), 0.0);
        let corner = marker_pin_inset(marker_pin_left(0.0));
        assert!(
            corner > 0.0 && corner < qol_theme::RADIUS_CONTROL,
            "{corner}"
        );
        assert_eq!(corner, marker_pin_inset(marker_pin_left(1.0)));
        assert_eq!(marker_thumb_left(0.0), 0.0);
        assert_eq!(marker_thumb_left(1.0) + MARKER_THUMB, width);
        assert_eq!(marker_thumb_left(0.5) + MARKER_THUMB / 2.0, width / 2.0);
        for fraction in [0.0, 0.02, 0.25, 0.5, 0.75, 0.98, 1.0] {
            let left = marker_thumb_left(fraction);
            assert!(
                left >= 0.0 && left + MARKER_THUMB <= width,
                "fraction {fraction}: thumb left {left}"
            );
        }
        assert_eq!(marker_thumb_top(), 22.0);
        assert_eq!(marker_thumb_top() + MARKER_THUMB, 34.0);
    }

    #[test]
    fn every_quarter_tick_sits_inside_the_track() {
        let width = SliderStyle::Wide.track_width();
        for at in TICKS {
            let left = tick_left(at, width);
            assert!(
                left >= 0.0 && left + qol_theme::LINE <= width,
                "tick {at}: left {left} in a {width} track"
            );
        }
        assert_eq!(tick_left(0.0, width), 0.0);
        assert_eq!(tick_left(1.0, width) + qol_theme::LINE, width);
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
