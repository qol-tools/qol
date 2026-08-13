use std::rc::Rc;

use gpui::prelude::*;
use gpui::{canvas, point, px, rgba, Bounds, Canvas, Path, PathBuilder, Pixels};
use qol_diff::constructs::ConstructKind;
use qol_diff::HeatLevel;

use crate::field::{AURORA_COLD, AURORA_EMBER, STAR_HOT};
use crate::view::LINE_HEIGHT;

pub const RAIL_PX: f32 = 22.0;
const RAIL_STROKE: f32 = 1.5;
const DEPTH_STEP_PX: f32 = 2.0;
const MAX_DEPTH_SHIFT_PX: f32 = 9.0;
const MIN_SPAN_PX: f32 = 4.0;
const BIRTH_ALPHA_PEAK: u32 = 0xd9;
const DEATH_DRIFT_PX: f32 = 8.0;
const COIL_TURNS_MIN: usize = 2;
const COIL_TURNS_MAX: usize = 8;
const RAIL_ALPHA_COOL: u32 = 0x66;
const RAIL_ALPHA_WARM: u32 = 0xb0;
const RAIL_ALPHA_HOT: u32 = 0xff;

pub struct RailSpec {
    pub seq: u64,
    pub morphing: bool,
    pub scroll: usize,
    pub old_scroll: usize,
    pub marks: Vec<RailMark>,
    pub deaths: Vec<RailDeath>,
}

pub struct RailMark {
    pub kind: ConstructKind,
    pub start: usize,
    pub end: usize,
    pub depth: usize,
    pub arms: usize,
    pub heat: HeatLevel,
    pub old: Option<RailGeo>,
}

pub struct RailGeo {
    pub start: usize,
    pub end: usize,
    pub arms: usize,
}

pub struct RailDeath {
    pub kind: ConstructKind,
    pub start: usize,
    pub end: usize,
    pub depth: usize,
    pub arms: usize,
    pub heat: HeatLevel,
}

pub struct RailPaint {
    layers: Vec<(Path<Pixels>, u32)>,
}

struct Geo {
    top: f32,
    bottom: f32,
    width: f32,
    count: usize,
}

pub fn rail_element(spec: Rc<RailSpec>, progress: f32) -> Canvas<RailPaint> {
    canvas(
        move |bounds, _, _| paint(&spec, progress, bounds),
        |_, paint, window, _| {
            for (path, color) in paint.layers {
                window.paint_path(path, rgba(color));
            }
        },
    )
    .flex_none()
    .w(px(RAIL_PX))
    .h_full()
}

fn paint(spec: &RailSpec, progress: f32, bounds: Bounds<Pixels>) -> RailPaint {
    let height = bounds.size.height.to_f64() as f32;
    let mut layers = Vec::new();
    for death in &spec.deaths {
        layers.extend(death_layers(spec, death, progress, height));
    }
    for mark in &spec.marks {
        layers.extend(mark_layers(spec, mark, progress, height));
    }
    RailPaint { layers }
}

fn mark_layers(
    spec: &RailSpec,
    mark: &RailMark,
    progress: f32,
    height: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let new_top = line_top(mark.start, spec.scroll);
    let new_bottom = line_top(mark.end + 1, spec.scroll);
    let birth = mark.old.is_none();
    let (top, bottom, old_count) = match &mark.old {
        Some(old) => (
            lerp(line_top(old.start, spec.old_scroll), new_top, progress),
            lerp(line_top(old.end + 1, spec.old_scroll), new_bottom, progress),
            old_count(mark, old),
        ),
        None => {
            let keyword = line_top(mark.start, spec.scroll);
            (
                lerp(keyword, new_top, progress),
                lerp(keyword, new_bottom, progress),
                mark_count(mark),
            )
        }
    };
    let width = (RAIL_PX - depth_shift(mark.depth)) * if birth { progress } else { 1.0 };
    let color = if birth {
        mix_hex(STAR_HOT, heat_color(mark.heat), progress)
    } else {
        heat_color(mark.heat)
    };
    let alpha = if birth {
        mix_u8(BIRTH_ALPHA_PEAK, heat_alpha(mark.heat), progress)
    } else {
        heat_alpha(mark.heat)
    };
    let extra_alpha = if birth {
        alpha
    } else {
        ((alpha as f32) * progress).round() as u32
    };
    let mut geo = Geo {
        top: top.clamp(0.0, height),
        bottom: bottom.clamp(0.0, height),
        width,
        count: mark_count(mark),
    };
    if geo.bottom - geo.top < MIN_SPAN_PX {
        geo.bottom = (geo.top + MIN_SPAN_PX).min(height);
    }
    kind_layers(
        mark.kind,
        &geo,
        old_count,
        color,
        alpha,
        extra_alpha,
        depth_shift(mark.depth),
    )
}

fn death_layers(
    spec: &RailSpec,
    death: &RailDeath,
    progress: f32,
    height: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let top = line_top(death.start, spec.old_scroll);
    let bottom = line_top(death.end + 1, spec.old_scroll);
    let keyword = line_top(death.start, spec.old_scroll);
    let drift = DEATH_DRIFT_PX * progress;
    let alpha = ((heat_alpha(death.heat) as f32) * (1.0 - progress)).round() as u32;
    let mut geo = Geo {
        top: (lerp(top, keyword, progress) - drift).clamp(0.0, height),
        bottom: (lerp(bottom, keyword, progress) - drift).clamp(0.0, height),
        width: (RAIL_PX - depth_shift(death.depth)) * (1.0 - progress),
        count: count_for(death.kind, death.arms, death.end - death.start + 1),
    };
    if geo.bottom - geo.top < MIN_SPAN_PX {
        geo.bottom = (geo.top + MIN_SPAN_PX).min(height);
    }
    kind_layers(
        death.kind,
        &geo,
        geo.count,
        heat_color(death.heat),
        alpha,
        alpha,
        depth_shift(death.depth),
    )
}

fn kind_layers(
    kind: ConstructKind,
    geo: &Geo,
    old_count: usize,
    color: u32,
    alpha: u32,
    extra_alpha: u32,
    shift: f32,
) -> Vec<(Path<Pixels>, u32)> {
    match kind {
        ConstructKind::Arc => arc_layers(geo, color, alpha, shift),
        ConstructKind::Coil => coil_layers(geo, old_count, color, alpha, extra_alpha, shift),
        ConstructKind::Fork => fork_layers(geo, old_count, color, alpha, extra_alpha, shift),
        ConstructKind::Lattice => lattice_layers(geo, old_count, color, alpha, extra_alpha, shift),
        ConstructKind::Tick => tick_layers(geo, color, alpha, shift),
    }
}

fn arc_layers(geo: &Geo, color: u32, alpha: u32, shift: f32) -> Vec<(Path<Pixels>, u32)> {
    let span = geo.bottom - geo.top;
    let x0 = geo.width * 0.18;
    let x1 = geo.width * 0.82;
    if span <= 0.0 || x1 - x0 <= 0.0 {
        return Vec::new();
    }
    let mut path = PathBuilder::stroke(px(RAIL_STROKE));
    path.move_to(point(px(x0), px(geo.bottom)));
    path.curve_to(
        point(px(geo.width * 0.5), px(geo.top)),
        point(px(x0), px(geo.top)),
    );
    path.curve_to(point(px(x1), px(geo.bottom)), point(px(x1), px(geo.top)));
    path.translate(point(px(shift), px(0.0)));
    match path.build() {
        Ok(path) => vec![(path, (color << 8) | alpha)],
        Err(_) => Vec::new(),
    }
}

fn coil_layers(
    geo: &Geo,
    old_count: usize,
    color: u32,
    alpha: u32,
    extra_alpha: u32,
    shift: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let turns = geo.count.clamp(COIL_TURNS_MIN, COIL_TURNS_MAX);
    let span = geo.bottom - geo.top;
    if span <= 0.0 || geo.width <= 0.0 {
        return Vec::new();
    }
    let step = span / turns as f32;
    let cx = geo.width * 0.5;
    let r0 = geo.width * 0.36;
    let mut layers = Vec::new();
    for turn in 0..turns {
        let rx = r0 * (1.0 - 0.55 * turn as f32 / turns as f32);
        let from = point(px(cx), px(geo.top + turn as f32 * step));
        let to = point(px(cx), px(geo.top + (turn + 1) as f32 * step));
        let mut path = PathBuilder::stroke(px(RAIL_STROKE));
        path.move_to(from);
        path.arc_to(
            point(px(rx), px(step * 0.5)),
            px(0.0),
            false,
            turn % 2 == 1,
            to,
        );
        path.translate(point(px(shift), px(0.0)));
        if let Ok(path) = path.build() {
            let part_alpha = if turn < old_count { alpha } else { extra_alpha };
            layers.push((path, (color << 8) | part_alpha));
        }
    }
    layers
}

fn fork_layers(
    geo: &Geo,
    old_count: usize,
    color: u32,
    alpha: u32,
    extra_alpha: u32,
    shift: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let span = geo.bottom - geo.top;
    if geo.width <= 0.0 {
        return Vec::new();
    }
    let cx = geo.width * 0.5;
    let mut layers = Vec::new();
    if span > 0.0 {
        let mut stem = PathBuilder::stroke(px(RAIL_STROKE));
        stem.move_to(point(px(cx), px(geo.top)));
        stem.line_to(point(px(cx), px(geo.bottom)));
        stem.translate(point(px(shift), px(0.0)));
        if let Ok(path) = stem.build() {
            layers.push((path, (color << 8) | alpha));
        }
    }
    let arms = geo.count.max(1);
    for arm in 0..arms {
        let y = geo.top + (arm as f32 + 0.5) * span / arms as f32;
        let mut prong = PathBuilder::stroke(px(RAIL_STROKE));
        prong.move_to(point(px(cx), px(y)));
        prong.line_to(point(px(geo.width * 0.92), px(y)));
        prong.translate(point(px(shift), px(0.0)));
        if let Ok(path) = prong.build() {
            let part_alpha = if arm < old_count { alpha } else { extra_alpha };
            layers.push((path, (color << 8) | part_alpha));
        }
    }
    layers
}

fn lattice_layers(
    geo: &Geo,
    old_count: usize,
    color: u32,
    alpha: u32,
    extra_alpha: u32,
    shift: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let span = geo.bottom - geo.top;
    if span <= 0.0 || geo.width <= 0.0 {
        return Vec::new();
    }
    let x0 = geo.width * 0.15;
    let x1 = geo.width * 0.85;
    let mut layers = Vec::new();
    let mut frame = PathBuilder::stroke(px(RAIL_STROKE));
    frame.move_to(point(px(x0), px(geo.top)));
    frame.line_to(point(px(x1), px(geo.top)));
    frame.line_to(point(px(x1), px(geo.bottom)));
    frame.line_to(point(px(x0), px(geo.bottom)));
    frame.line_to(point(px(x0), px(geo.top)));
    frame.translate(point(px(shift), px(0.0)));
    if let Ok(path) = frame.build() {
        layers.push((path, (color << 8) | alpha));
    }
    for column in [geo.width * 0.45, geo.width * 0.63] {
        let mut vertical = PathBuilder::stroke(px(RAIL_STROKE));
        vertical.move_to(point(px(column), px(geo.top)));
        vertical.line_to(point(px(column), px(geo.bottom)));
        vertical.translate(point(px(shift), px(0.0)));
        if let Ok(path) = vertical.build() {
            layers.push((path, (color << 8) | alpha));
        }
    }
    let rows = geo.count;
    if rows > 0 {
        let row_step = span / (rows + 2) as f32;
        for row in 1..=rows {
            let y = geo.top + row as f32 * row_step;
            let mut line = PathBuilder::stroke(px(RAIL_STROKE));
            line.move_to(point(px(x0), px(y)));
            line.line_to(point(px(x1), px(y)));
            line.translate(point(px(shift), px(0.0)));
            if let Ok(path) = line.build() {
                let part_alpha = if row <= old_count { alpha } else { extra_alpha };
                layers.push((path, (color << 8) | part_alpha));
            }
        }
    }
    layers
}

fn tick_layers(geo: &Geo, color: u32, alpha: u32, shift: f32) -> Vec<(Path<Pixels>, u32)> {
    if geo.width <= 0.0 {
        return Vec::new();
    }
    let y = (geo.top + geo.bottom) * 0.5;
    let mut path = PathBuilder::stroke(px(RAIL_STROKE));
    path.move_to(point(px(geo.width * 0.30), px(y + 3.0)));
    path.line_to(point(px(geo.width * 0.48), px(y - 2.0)));
    path.line_to(point(px(geo.width * 0.74), px(y + 2.0)));
    path.translate(point(px(shift), px(0.0)));
    match path.build() {
        Ok(path) => vec![(path, (color << 8) | alpha)],
        Err(_) => Vec::new(),
    }
}

fn mark_count(mark: &RailMark) -> usize {
    count_for(mark.kind, mark.arms, mark.end - mark.start + 1)
}

fn old_count(mark: &RailMark, old: &RailGeo) -> usize {
    count_for(mark.kind, old.arms, old.end - old.start + 1)
}

fn count_for(kind: ConstructKind, arms: usize, span_lines: usize) -> usize {
    match kind {
        ConstructKind::Fork => arms.max(1),
        ConstructKind::Coil => span_lines.clamp(COIL_TURNS_MIN, COIL_TURNS_MAX),
        ConstructKind::Lattice => span_lines.saturating_sub(2),
        ConstructKind::Arc | ConstructKind::Tick => 0,
    }
}

fn line_top(line: usize, scroll: usize) -> f32 {
    (line as f32 - scroll as f32) * LINE_HEIGHT
}

fn depth_shift(depth: usize) -> f32 {
    (depth as f32 * DEPTH_STEP_PX).min(MAX_DEPTH_SHIFT_PX)
}

fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

fn heat_color(heat: HeatLevel) -> u32 {
    match heat {
        HeatLevel::Cool => AURORA_COLD,
        HeatLevel::Warm => AURORA_EMBER,
        HeatLevel::Hot => STAR_HOT,
    }
}

fn heat_alpha(heat: HeatLevel) -> u32 {
    match heat {
        HeatLevel::Cool => RAIL_ALPHA_COOL,
        HeatLevel::Warm => RAIL_ALPHA_WARM,
        HeatLevel::Hot => RAIL_ALPHA_HOT,
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

fn mix_u8(from: u32, to: u32, amount: f32) -> u32 {
    let value = from as f32 + (to as f32 - from as f32) * amount.clamp(0.0, 1.0);
    value.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_ramp_walks_cool_to_ember_to_white_hot() {
        assert_eq!(heat_color(HeatLevel::Cool), AURORA_COLD);
        assert_eq!(heat_color(HeatLevel::Warm), AURORA_EMBER);
        assert_eq!(heat_color(HeatLevel::Hot), STAR_HOT);
        assert!(heat_alpha(HeatLevel::Cool) < heat_alpha(HeatLevel::Warm));
        assert!(heat_alpha(HeatLevel::Warm) < heat_alpha(HeatLevel::Hot));
    }

    #[test]
    fn depth_shift_is_bounded() {
        assert_eq!(depth_shift(0), 0.0);
        assert_eq!(depth_shift(1), 2.0);
        assert_eq!(depth_shift(3), 6.0);
        assert_eq!(depth_shift(20), MAX_DEPTH_SHIFT_PX);
        assert!(RAIL_PX - depth_shift(20) > 0.0, "the rail never collapses");
    }

    #[test]
    fn line_top_counts_rows_from_the_scroll_offset() {
        assert_eq!(line_top(0, 0), 0.0);
        assert_eq!(line_top(4, 0), 4.0 * LINE_HEIGHT);
        assert_eq!(line_top(4, 2), 2.0 * LINE_HEIGHT);
        assert_eq!(line_top(2, 4), -2.0 * LINE_HEIGHT);
    }

    #[test]
    fn count_for_maps_constructs_to_discrete_parts() {
        assert_eq!(count_for(ConstructKind::Fork, 4, 12), 4);
        assert_eq!(count_for(ConstructKind::Fork, 0, 12), 1);
        assert_eq!(count_for(ConstructKind::Coil, 0, 1), 2);
        assert_eq!(count_for(ConstructKind::Coil, 0, 20), 8);
        assert_eq!(count_for(ConstructKind::Coil, 0, 5), 5);
        assert_eq!(count_for(ConstructKind::Lattice, 0, 4), 2);
        assert_eq!(count_for(ConstructKind::Lattice, 0, 1), 0);
        assert_eq!(count_for(ConstructKind::Arc, 0, 9), 0);
        assert_eq!(count_for(ConstructKind::Tick, 0, 1), 0);
    }

    #[test]
    fn lerp_blends_between_two_geometries() {
        assert_eq!(lerp(10.0, 20.0, 0.0), 10.0);
        assert_eq!(lerp(10.0, 20.0, 1.0), 20.0);
        assert_eq!(lerp(10.0, 20.0, 0.5), 15.0);
    }
}
