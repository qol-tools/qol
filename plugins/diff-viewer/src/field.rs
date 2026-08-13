use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, ease_out_quint, linear_color_stop, linear_gradient, point, px, rgb, rgba, Animation,
    AnimationExt as _, AnyElement, BoxShadow, DefiniteLength, Div, ElementId, LinearColorStop,
};

use crate::rail::{self, RailSpec};
use crate::scrubber::Commit;
use crate::view::{LINE_HEIGHT, TRANSITION_MS};

pub const FIELD_TOP: u32 = 0x0a0d1a;
pub const FIELD_BOTTOM: u32 = 0x12162b;
pub const AURORA_COLD: u32 = 0x2a3a6e;
pub const AURORA_EMBER: u32 = 0xff8c42;
pub const AURORA_WARM: u32 = 0xffd9a0;
pub const STAR_OLDEST: u32 = 0x3a435c;
pub const STAR_HOT: u32 = 0xfff2e0;
pub const ASH_START: u32 = AURORA_EMBER;
pub const ASH_END: u32 = 0x2a2430;

const CHROME_RESERVED_PX: f32 = 62.0;
const HORIZON_FRACTION: f32 = 0.66;
const HORIZON_FROM_BOTTOM: f32 = 1.0 - HORIZON_FRACTION;
const AURORA_PERIOD_S: f32 = 2.5;
const AURORA_BASE_PX: f32 = 56.0;
const AURORA_SWING_MIN_PX: f32 = 18.0;
const AURORA_SWING_SCALE_PX: f32 = 34.0;
const AURORA_OPACITY_BASE: f32 = 0.30;
const AURORA_OPACITY_SWING: f32 = 0.10;
const AURORA_OVERLAY_FRACTION: f32 = 0.45;
const AURORA_CORNER_PX: f32 = 12.0;
const RECENT_COMMITS: usize = 8;
const MAGNITUDE_CLAMP: u64 = 5_000;
const RIBBON_TOP_FRACTION: f32 = 0.10;
const RIBBON_HEIGHT_FRACTION: f32 = 0.50;
const RIBBON_WIDTH_FRACTION: f32 = 0.72;
const RIBBON_MARGIN_FRACTION: f32 = (1.0 - RIBBON_WIDTH_FRACTION) / 2.0;
const RIBBON_CORNER_PX: f32 = 10.0;
const RIBBON_BLOOM_ALPHA: u32 = 0x1a;
const STAR_SLOT_PX: f32 = 18.0;
const STAR_MIN_PX: f32 = 4.0;
const STAR_RANGE_PX: f32 = 8.0;
const STAR_EDGE_FRACTION: f32 = 0.03;
const STAR_BRIGHTEN: f32 = 0.30;
const CORONA_WIDE_PX: f32 = 46.0;
const CORONA_TIGHT_PX: f32 = 26.0;
const CORONA_WIDE_ALPHA: f32 = 0.16;
const CORONA_TIGHT_ALPHA: f32 = 0.38;
const CORONA_BREATHE: f32 = 0.35;
const CORONA_MIN_ALPHA: f32 = 0.35;
const ASH_MIN: usize = 12;
const ASH_SPAN: usize = 9;
const ASH_SIZE_MIN_PX: f32 = 2.0;
const ASH_SIZE_SPAN_PX: f32 = 2.0;
const ASH_RISE_PX: f32 = 8.0;
const ASH_FALL_PX: f32 = 40.0;
const ASH_RISE_UNTIL: f32 = 0.3;
const ASH_JITTER_PX: f32 = 10.0;
const ASH_ALPHA: u32 = 0xcc;
const CONE_MIN_FRACTION: f32 = 0.015;
const CONE_ALPHA: f32 = 0.45;
const CONE_EDGE_ALPHA: u32 = 0xb0;

pub struct AshSpec {
    pub seq: u64,
    pub removed_rows: Vec<f32>,
}

pub struct ConeSpec {
    pub seq: u64,
    pub from: usize,
    pub to: usize,
}

pub struct SceneState {
    pub pane_height: f32,
    pub phase_seconds: f32,
    pub ash: Option<AshSpec>,
    pub cone: Option<ConeSpec>,
    pub rail: Option<Rc<RailSpec>>,
    pub bloom_rank: u8,
}

pub fn background() -> gpui::Background {
    linear_gradient(0.0, stop(rgb(FIELD_TOP)), stop(rgb(FIELD_BOTTOM)))
}

pub fn pane_height(viewport_height: f32) -> f32 {
    (viewport_height - CHROME_RESERVED_PX).max(0.0)
}

pub fn ribbon_fit(pane_height: f32) -> usize {
    ((pane_height * RIBBON_HEIGHT_FRACTION) / LINE_HEIGHT)
        .floor()
        .max(1.0) as usize
}

pub fn bloom_alpha(rank: u8) -> u32 {
    RIBBON_BLOOM_ALPHA * rank as u32 / 2
}

pub fn scene(
    rows: Vec<AnyElement>,
    commits: &[Commit],
    selected: usize,
    state: SceneState,
) -> AnyElement {
    let phase = state.phase_seconds * std::f32::consts::TAU / AURORA_PERIOD_S;
    let breath = 0.5 + 0.5 * phase.sin();
    let scale = magnitude_scale(commits);
    let aurora_height =
        AURORA_BASE_PX + (AURORA_SWING_MIN_PX + AURORA_SWING_SCALE_PX * scale) * breath;
    let aurora_opacity = AURORA_OPACITY_BASE + AURORA_OPACITY_SWING * breath;
    let mut scene = div().relative().size_full();
    scene = scene.child(aurora(aurora_height, aurora_opacity));
    for index in 0..commits.len() {
        scene = scene.child(star(commits, index, selected, phase));
    }
    if !rows.is_empty() {
        scene = scene.child(ribbon(rows, state.rail, bloom_alpha(state.bloom_rank)));
    }
    if let Some(ash) = state.ash {
        scene = scene.child(ash_layer(&ash, state.pane_height));
    }
    if let Some(cone) = state.cone {
        scene = scene.child(light_cone(&cone, commits.len()));
    }
    scene.into_any_element()
}

fn aurora(height: f32, opacity: f32) -> Div {
    div()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .bottom(fraction(HORIZON_FROM_BOTTOM))
        .h(px(height))
        .rounded(px(AURORA_CORNER_PX))
        .bg(linear_gradient(
            90.0,
            stop(rgb(AURORA_COLD)),
            stop(rgb(AURORA_EMBER)),
        ))
        .opacity(opacity)
        .child(
            div()
                .absolute()
                .left(fraction(1.0 - AURORA_OVERLAY_FRACTION))
                .w(fraction(AURORA_OVERLAY_FRACTION))
                .h_full()
                .rounded(px(AURORA_CORNER_PX))
                .bg(linear_gradient(
                    90.0,
                    stop(rgb(AURORA_EMBER)),
                    stop(rgb(AURORA_WARM)),
                )),
        )
}

fn star(commits: &[Commit], index: usize, selected: usize, phase: f32) -> Div {
    let commit = &commits[index];
    let t = heat_t(index, commits.len());
    let brightness = magnitude_curve(commit.magnitude as f32);
    let hue = heat_hue(t);
    let color = mix_hex(hue, STAR_HOT, brightness * STAR_BRIGHTEN);
    let size = STAR_MIN_PX + STAR_RANGE_PX * brightness;
    let mut slot = div()
        .absolute()
        .left(fraction(star_x(index, commits.len())))
        .bottom(fraction(HORIZON_FROM_BOTTOM))
        .w(px(STAR_SLOT_PX))
        .h(px(STAR_SLOT_PX))
        .flex()
        .items_center()
        .justify_center();
    if index == selected {
        let glow = CORONA_WIDE_ALPHA
            * (CORONA_MIN_ALPHA + (1.0 - CORONA_MIN_ALPHA) * brightness)
            * (1.0 - CORONA_BREATHE + CORONA_BREATHE * phase.sin().abs());
        let core = CORONA_TIGHT_ALPHA
            * (CORONA_MIN_ALPHA + (1.0 - CORONA_MIN_ALPHA) * brightness)
            * (1.0 - CORONA_BREATHE + CORONA_BREATHE * phase.sin().abs());
        slot = slot
            .child(corona_ring(
                (CORONA_WIDE_PX - STAR_SLOT_PX) / 2.0,
                CORONA_WIDE_PX,
                AURORA_EMBER,
                glow,
            ))
            .child(corona_ring(
                (CORONA_TIGHT_PX - STAR_SLOT_PX) / 2.0,
                CORONA_TIGHT_PX,
                STAR_HOT,
                core,
            ));
    }
    slot.child(div().w(px(size)).h(px(size)).rounded_full().bg(rgb(color)))
}

fn corona_ring(offset: f32, size: f32, color: u32, alpha: f32) -> Div {
    div()
        .absolute()
        .left(px(-offset))
        .top(px(-offset))
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .bg(rgba((color << 8) | alpha_byte(alpha)))
}

fn ribbon(rows: Vec<AnyElement>, rail: Option<Rc<RailSpec>>, bloom_alpha: u32) -> Div {
    let row_count = rows.len();
    let content = match rail {
        Some(spec) => {
            let element = rail::rail_element(Rc::clone(&spec), 1.0);
            let element = if spec.morphing {
                element
                    .with_animation(
                        ElementId::named_usize(format!("dw-rail-{}", spec.seq), 0),
                        Animation::new(TRANSITION_MS).with_easing(ease_out_quint()),
                        move |_, delta| rail::rail_element(Rc::clone(&spec), delta),
                    )
                    .into_any_element()
            } else {
                element.into_any_element()
            };
            div()
                .flex()
                .flex_row()
                .size_full()
                .overflow_hidden()
                .child(element)
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .children(rows),
                )
        }
        None => div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .children(rows),
    };
    let mut ribbon = div()
        .absolute()
        .top(fraction(RIBBON_TOP_FRACTION))
        .left(fraction(RIBBON_MARGIN_FRACTION))
        .w(fraction(RIBBON_WIDTH_FRACTION))
        .h(px(row_count as f32 * LINE_HEIGHT));
    if bloom_alpha > 0 {
        ribbon = ribbon.child(
            div()
                .absolute()
                .size_full()
                .rounded(px(RIBBON_CORNER_PX))
                .shadow(vec![BoxShadow {
                    color: rgba((AURORA_EMBER << 8) | bloom_alpha).into(),
                    offset: point(px(0.0), px(0.0)),
                    blur_radius: px(48.0),
                    spread_radius: px(16.0),
                }]),
        );
    }
    ribbon.child(content)
}

fn ash_layer(spec: &AshSpec, pane_height: f32) -> Div {
    let count = ash_count(spec.seq);
    let ribbon_top = pane_height * RIBBON_TOP_FRACTION;
    let mut layer = div().absolute().size_full();
    for index in 0..count {
        let seed = particle_seed(spec.seq, index);
        let row = spec.removed_rows[index % spec.removed_rows.len()];
        let x = unit(seed);
        let y = ribbon_top + row + (unit(seed.rotate_left(17)) * 2.0 - 1.0) * ASH_JITTER_PX;
        let size = ASH_SIZE_MIN_PX + unit(seed.rotate_left(41)) * ASH_SIZE_SPAN_PX;
        layer = layer.child(ash_particle(spec.seq, index, x, y, size));
    }
    layer
}

fn ash_particle(seq: u64, index: usize, x: f32, y: f32, size: f32) -> AnyElement {
    div()
        .absolute()
        .left(fraction(x))
        .top(px(y))
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .bg(rgba((ASH_START << 8) | ASH_ALPHA))
        .with_animation(
            ElementId::named_usize(format!("dw-ash-{seq}"), index),
            Animation::new(TRANSITION_MS),
            move |element, progress| {
                let color = mix_hex(ASH_START, ASH_END, progress);
                element
                    .top(px(y + ash_drift(progress)))
                    .bg(rgba((color << 8) | alpha_byte(1.0 - progress)))
                    .opacity(1.0 - progress)
            },
        )
        .into_any_element()
}

fn light_cone(spec: &ConeSpec, commit_len: usize) -> AnyElement {
    if commit_len <= 1 {
        return div().into_any_element();
    }
    let from = star_x(spec.from.min(commit_len - 1), commit_len);
    let to = star_x(spec.to.min(commit_len - 1), commit_len);
    if (from - to).abs() < 0.001 {
        return div().into_any_element();
    }
    let rightward = to > from;
    let gradient = if rightward {
        linear_gradient(90.0, cone_edge(true), cone_edge(false))
    } else {
        linear_gradient(270.0, cone_edge(true), cone_edge(false))
    };
    div()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .bg(gradient)
        .with_animation(
            ElementId::named_usize(format!("dw-cone-{}", spec.seq), 0),
            Animation::new(TRANSITION_MS).with_easing(ease_out_quint()),
            move |element, delta| {
                let edge = from + (to - from) * delta;
                element
                    .left(fraction(from.min(edge)))
                    .w(fraction((edge - from).abs() + CONE_MIN_FRACTION))
                    .opacity(CONE_ALPHA * (1.0 - delta))
            },
        )
        .into_any_element()
}

fn cone_edge(transparent: bool) -> LinearColorStop {
    if transparent {
        stop_at(rgba(0x00000000), 0.0)
    } else {
        stop_at(rgba((AURORA_EMBER << 8) | CONE_EDGE_ALPHA), 1.0)
    }
}

fn stop(color: gpui::Rgba) -> LinearColorStop {
    stop_at(color, 0.0)
}

fn stop_at(color: gpui::Rgba, percentage: f32) -> LinearColorStop {
    linear_color_stop(color, percentage)
}

fn fraction(value: f32) -> DefiniteLength {
    DefiniteLength::Fraction(value)
}

fn heat_t(index: usize, len: usize) -> f32 {
    if len <= 1 {
        1.0
    } else {
        1.0 - index as f32 / (len - 1) as f32
    }
}

fn star_x(index: usize, len: usize) -> f32 {
    if len <= 1 {
        0.5
    } else {
        STAR_EDGE_FRACTION + heat_t(index, len) * (1.0 - 2.0 * STAR_EDGE_FRACTION)
    }
}

fn heat_hue(t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        mix_hex(STAR_OLDEST, AURORA_EMBER, t * 2.0)
    } else {
        mix_hex(AURORA_EMBER, STAR_HOT, (t - 0.5) * 2.0)
    }
}

fn magnitude_scale(commits: &[Commit]) -> f32 {
    let count = commits.len().min(RECENT_COMMITS);
    if count == 0 {
        return 0.0;
    }
    let total: u64 = commits
        .iter()
        .take(RECENT_COMMITS)
        .map(|commit| commit.magnitude)
        .sum();
    magnitude_curve(total as f32 / count as f32)
}

fn magnitude_curve(magnitude: f32) -> f32 {
    let clamped = magnitude.min(MAGNITUDE_CLAMP as f32).max(0.0);
    clamped.ln_1p() / (MAGNITUDE_CLAMP as f32).ln_1p()
}

fn ash_drift(progress: f32) -> f32 {
    if progress < ASH_RISE_UNTIL {
        -ASH_RISE_PX * progress / ASH_RISE_UNTIL
    } else {
        let fall = (progress - ASH_RISE_UNTIL) / (1.0 - ASH_RISE_UNTIL);
        -ASH_RISE_PX + ASH_FALL_PX * fall * fall
    }
}

fn mix_hex(from: u32, to: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let a = (from >> shift) & 0xff;
        let b = (to >> shift) & 0xff;
        let value = a as f32 + (b as f32 - a as f32) * amount.clamp(0.0, 1.0);
        ((value.round() as u32) & 0xff) << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn alpha_byte(alpha: f32) -> u32 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u32
}

fn ash_count(seq: u64) -> usize {
    ASH_MIN + (seq % ASH_SPAN as u64) as usize
}

fn particle_seed(seq: u64, index: usize) -> u64 {
    let mut z = seq.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (index as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(seed: u64) -> f32 {
    (seed & 0x00FF_FFFF) as f32 / 0x0100_0000 as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(magnitude: u64) -> Commit {
        Commit::with_magnitude("sha", "subject", magnitude)
    }

    fn commits(magnitudes: &[u64]) -> Vec<Commit> {
        magnitudes.iter().copied().map(commit).collect()
    }

    #[test]
    fn newest_commit_sits_at_the_present_edge() {
        assert!((star_x(0, 5) - (1.0 - STAR_EDGE_FRACTION)).abs() < 0.001);
        assert_eq!(star_x(4, 5), STAR_EDGE_FRACTION);
        assert_eq!(star_x(0, 1), 0.5);
        assert!(star_x(0, 5) > star_x(1, 5));
        assert!(star_x(1, 5) > star_x(2, 5));
    }

    #[test]
    fn star_hue_walks_blue_to_ember_to_white_hot() {
        assert_eq!(heat_hue(1.0), STAR_HOT, "the newest commit is white-hot");
        assert_eq!(heat_hue(0.0), STAR_OLDEST, "the oldest commit is cold blue");
        assert_eq!(heat_hue(0.5), AURORA_EMBER);
        let cool = heat_hue(0.25);
        assert!(
            (cool >> 16) & 0xff < (AURORA_EMBER >> 16) & 0xff,
            "the cool half stays below ember red"
        );
        let hot = heat_hue(0.75);
        assert!(hot >= AURORA_EMBER, "the hot half burns at or past ember");
    }

    #[test]
    fn magnitude_curve_reuses_the_scrubber_log_curve() {
        assert_eq!(magnitude_curve(0.0), 0.0);
        assert_eq!(magnitude_curve(MAGNITUDE_CLAMP as f32), 1.0);
        assert_eq!(magnitude_curve(MAGNITUDE_CLAMP as f32 * 10.0), 1.0);
        assert!(magnitude_curve(10.0) > magnitude_curve(5.0));
        assert!(magnitude_curve(100.0) > magnitude_curve(10.0));
        assert!(
            magnitude_curve(11.0) - magnitude_curve(10.0)
                < magnitude_curve(6.0) - magnitude_curve(5.0),
            "adjacent-unit gains shrink as magnitude grows"
        );
        let scrubber_t = |magnitude: f32| {
            let clamped = magnitude.min(MAGNITUDE_CLAMP as f32);
            clamped.ln_1p() / (MAGNITUDE_CLAMP as f32).ln_1p()
        };
        for magnitude in [0.0, 1.0, 7.0, 42.0, 5000.0] {
            assert_eq!(magnitude_curve(magnitude), scrubber_t(magnitude));
        }
    }

    #[test]
    fn aurora_amplitude_scales_with_recent_magnitudes() {
        assert_eq!(magnitude_scale(&[]), 0.0);
        assert_eq!(magnitude_scale(&commits(&[0, 0, 0])), 0.0);
        let quiet = magnitude_scale(&commits(&[1, 2, 1]));
        let loud = magnitude_scale(&commits(&[4000, 4000, 4000]));
        assert!(loud > quiet, "bigger recent commits raise the aurora swing");
        let loudest = magnitude_scale(&commits(&[MAGNITUDE_CLAMP; 8]));
        assert!((loudest - 1.0).abs() < 0.001);
    }

    #[test]
    fn ash_counts_stay_in_the_twelve_to_twenty_range() {
        for seq in 0..64 {
            let count = ash_count(seq);
            assert!((ASH_MIN..=ASH_MIN + ASH_SPAN - 1).contains(&count));
        }
        assert_ne!(ash_count(0), ash_count(1));
    }

    #[test]
    fn ash_particles_are_deterministic_per_transition() {
        for index in 0..20 {
            assert_eq!(
                particle_seed(7, index),
                particle_seed(7, index),
                "the same particle in the same transition is stable"
            );
        }
        assert_ne!(particle_seed(7, 0), particle_seed(8, 0));
        assert_ne!(particle_seed(7, 0), particle_seed(7, 1));
    }

    #[test]
    fn ash_drift_rises_briefly_then_falls_with_gravity() {
        assert_eq!(ash_drift(0.0), 0.0);
        assert!(ash_drift(0.15) < 0.0, "the particle first drifts up");
        assert!(ash_drift(0.29) < ash_drift(0.15));
        assert!(ash_drift(1.0) > 0.0, "the particle ends below its start");
        assert!(
            ash_drift(1.0) - ash_drift(0.75) > ash_drift(0.75) - ash_drift(0.5),
            "the fall accelerates like gravity"
        );
    }

    #[test]
    fn cone_spans_from_old_star_to_new_star() {
        let from = star_x(0, 6);
        let to = star_x(3, 6);
        assert!(from > to, "newer stars sit right of older ones");
        let span = |delta: f32| ((to - from) * delta).abs();
        assert!(span(0.0) < 0.001, "the cone starts at the old star");
        assert!(
            (span(1.0) - (from - to)).abs() < 0.001,
            "the cone reaches the new star"
        );
        assert!(span(0.5) > 0.0);
    }

    #[test]
    fn ribbon_fit_respects_pane_geometry() {
        let fit = ribbon_fit(578.0);
        assert!(fit >= 1);
        assert_eq!(fit as f32 * LINE_HEIGHT, 288.0);
        assert!(ribbon_fit(0.0) >= 1, "a degenerate pane still shows a row");
    }

    #[test]
    fn mix_hex_interpolates_each_channel() {
        assert_eq!(mix_hex(0x000000, 0xffffff, 0.5), 0x808080);
        assert_eq!(mix_hex(0xff8c42, 0x2a2430, 1.0), 0x2a2430);
        assert_eq!(mix_hex(0xff8c42, 0x2a2430, 0.0), 0xff8c42);
        assert_eq!(mix_hex(0x112233, 0x112233, 0.75), 0x112233);
    }

    #[test]
    fn alpha_byte_clamps() {
        assert_eq!(alpha_byte(0.0), 0);
        assert_eq!(alpha_byte(1.0), 255);
        assert_eq!(alpha_byte(2.0), 255);
        assert_eq!(alpha_byte(-1.0), 0);
        assert_eq!(alpha_byte(0.5), 128);
    }

    #[test]
    fn scene_builds_from_any_state() {
        let _empty = scene(
            Vec::new(),
            &[],
            0,
            SceneState {
                pane_height: 578.0,
                phase_seconds: 0.0,
                ash: None,
                cone: None,
                rail: None,
                bloom_rank: 0,
            },
        );
        let _full = scene(
            Vec::new(),
            &commits(&[1, 2, 3]),
            1,
            SceneState {
                pane_height: 578.0,
                phase_seconds: 3.7,
                ash: Some(AshSpec {
                    seq: 4,
                    removed_rows: vec![0.0, 54.0],
                }),
                cone: Some(ConeSpec {
                    seq: 4,
                    from: 0,
                    to: 2,
                }),
                rail: Some(Rc::new(RailSpec {
                    seq: 4,
                    morphing: true,
                    scroll: 0,
                    old_scroll: 0,
                    marks: Vec::new(),
                    deaths: Vec::new(),
                })),
                bloom_rank: 2,
            },
        );
        let _both = scene(
            Vec::new(),
            &commits(&[1, 2, 3]),
            1,
            SceneState {
                pane_height: 578.0,
                phase_seconds: 3.7,
                ash: Some(AshSpec {
                    seq: 4,
                    removed_rows: vec![0.0],
                }),
                cone: None,
                rail: None,
                bloom_rank: 1,
            },
        );
    }

    #[test]
    fn bloom_alpha_is_strictly_proportional_to_heat_rank() {
        assert_eq!(bloom_alpha(0), 0, "all cool renders no bloom");
        assert_eq!(bloom_alpha(2), RIBBON_BLOOM_ALPHA);
        assert_eq!(bloom_alpha(1) * 2, bloom_alpha(2), "warm sits halfway");
    }
}
