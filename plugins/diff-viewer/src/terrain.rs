use std::rc::Rc;

use gpui::prelude::*;
use gpui::{canvas, point, px, rgba, Bounds, Canvas, Path, PathBuilder, Pixels};
use qol_diff::constructs::ConstructKind;
use qol_diff::HeatLevel;

use crate::field::{AURORA_COLD, AURORA_EMBER, STAR_HOT};

pub const TERRAIN_HEIGHT_FRACTION: f32 = 0.20;
const TERRAIN_STROKE: f32 = 1.5;
const DEPTH_STEP_PX: f32 = 8.0;
const MAX_DEPTH_SHIFT_PX: f32 = 28.0;
const DEPTH_DIM: f32 = 0.10;
const MIN_DEPTH_DIM: f32 = 0.40;
const MIN_SPAN_PX: f32 = 6.0;
const BIRTH_ALPHA_PEAK: u32 = 0xd9;
const COIL_TURNS_MIN: usize = 2;
const COIL_TURNS_MAX: usize = 8;
const ALPHA_COOL: u32 = 0x66;
const ALPHA_WARM: u32 = 0xb0;
const ALPHA_HOT: u32 = 0xff;
const ARC_HEIGHT_PX: f32 = 56.0;
const COIL_HEIGHT_PX: f32 = 42.0;
const FORK_HEIGHT_PX: f32 = 36.0;
const LATTICE_HEIGHT_PX: f32 = 46.0;
const TICK_HEIGHT_PX: f32 = 18.0;
const PULSE_PERIOD_S: f32 = 2.5;
const PULSE_AMPLITUDE: f32 = 0.08;

pub struct TerrainSpec {
    pub seq: u64,
    pub morphing: bool,
    pub rows: usize,
    pub marks: Vec<TerrainMark>,
    pub deaths: Vec<TerrainDeath>,
}

pub struct TerrainMark {
    pub kind: ConstructKind,
    pub start: usize,
    pub end: usize,
    pub depth: usize,
    pub arms: usize,
    pub heat: HeatLevel,
    pub old: Option<TerrainGeo>,
}

pub struct TerrainGeo {
    pub start: usize,
    pub end: usize,
    pub arms: usize,
    pub rows: usize,
}

pub struct TerrainDeath {
    pub kind: ConstructKind,
    pub start: usize,
    pub end: usize,
    pub depth: usize,
    pub arms: usize,
    pub heat: HeatLevel,
    pub rows: usize,
}

pub struct TerrainPaint {
    layers: Vec<(Path<Pixels>, u32)>,
}

struct Geo {
    x0: f32,
    x1: f32,
    top: f32,
    floor: f32,
    count: usize,
}

fn offset_geo(geo: Geo, ox: f32, oy: f32) -> Geo {
    Geo {
        x0: geo.x0 + ox,
        x1: geo.x1 + ox,
        top: geo.top + oy,
        floor: geo.floor + oy,
        count: geo.count,
    }
}

pub fn terrain_element(spec: Rc<TerrainSpec>, progress: f32, phase: f32) -> Canvas<TerrainPaint> {
    canvas(
        move |bounds, _, _| paint(&spec, progress, phase, bounds),
        |_, paint, window, _| {
            for (path, color) in paint.layers {
                window.paint_path(path, rgba(color));
            }
        },
    )
    .size_full()
}

fn paint(spec: &TerrainSpec, progress: f32, phase: f32, bounds: Bounds<Pixels>) -> TerrainPaint {
    let width = bounds.size.width.to_f64() as f32;
    let height = bounds.size.height.to_f64() as f32;
    let ox = bounds.origin.x.to_f64() as f32;
    let oy = bounds.origin.y.to_f64() as f32;
    let mut layers = Vec::new();
    for death in &spec.deaths {
        layers.extend(death_layers(death, progress, phase, width, height, ox, oy));
    }
    for mark in &spec.marks {
        layers.extend(mark_layers(
            spec, mark, progress, phase, width, height, ox, oy,
        ));
    }
    TerrainPaint { layers }
}

#[allow(clippy::too_many_arguments)]
fn mark_layers(
    spec: &TerrainSpec,
    mark: &TerrainMark,
    progress: f32,
    phase: f32,
    width: f32,
    height: f32,
    ox: f32,
    oy: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let (new_x0, new_x1) = band_span(mark.start, mark.end, spec.rows, width);
    let birth = mark.old.is_none();
    let (x0, x1, top, old_count) = match &mark.old {
        Some(old) => {
            let (old_x0, old_x1) = band_span(old.start, old.end, old.rows, width);
            (
                lerp(old_x0, new_x0, progress),
                lerp(old_x1, new_x1, progress),
                height - glyph_height(mark.kind, mark.depth),
                old_count(mark, old),
            )
        }
        None => {
            let keyword = band_x(mark.start, spec.rows, width);
            (
                lerp(keyword, new_x0, progress),
                lerp(keyword, new_x1, progress),
                height - glyph_height(mark.kind, mark.depth) * progress,
                mark_count(mark),
            )
        }
    };
    let (x0, x1) = min_span(x0, x1, width);
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
    let alpha = breathe(alpha, mark.heat, phase);
    let alpha = (alpha as f32 * depth_dim(mark.depth)).round() as u32;
    let extra_alpha = if birth {
        alpha
    } else {
        ((alpha as f32) * progress).round() as u32
    };
    let geo = offset_geo(
        Geo {
            x0: x0.clamp(0.0, width),
            x1: x1.clamp(0.0, width),
            top: top.clamp(0.0, height),
            floor: height,
            count: mark_count(mark),
        },
        ox,
        oy,
    );
    kind_layers(mark.kind, &geo, old_count, color, alpha, extra_alpha)
}

fn death_layers(
    death: &TerrainDeath,
    progress: f32,
    phase: f32,
    width: f32,
    height: f32,
    ox: f32,
    oy: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let (old_x0, old_x1) = band_span(death.start, death.end, death.rows, width);
    let keyword = band_x(death.start, death.rows, width);
    let alpha = heat_alpha(death.heat);
    let alpha = breathe(alpha, death.heat, phase);
    let alpha = (alpha as f32 * depth_dim(death.depth) * (1.0 - progress)).round() as u32;
    let (x0, x1) = min_span(
        lerp(old_x0, keyword, progress),
        lerp(old_x1, keyword, progress),
        width,
    );
    let geo = offset_geo(
        Geo {
            x0: x0.clamp(0.0, width),
            x1: x1.clamp(0.0, width),
            top: (height - glyph_height(death.kind, death.depth) * (1.0 - progress))
                .clamp(0.0, height),
            floor: height,
            count: count_for(death.kind, death.arms, death.end - death.start + 1),
        },
        ox,
        oy,
    );
    kind_layers(
        death.kind,
        &geo,
        geo.count,
        heat_color(death.heat),
        alpha,
        alpha,
    )
}

fn kind_layers(
    kind: ConstructKind,
    geo: &Geo,
    old_count: usize,
    color: u32,
    alpha: u32,
    extra_alpha: u32,
) -> Vec<(Path<Pixels>, u32)> {
    match kind {
        ConstructKind::Arc => arc_layers(geo, color, alpha),
        ConstructKind::Coil => coil_layers(geo, old_count, color, alpha, extra_alpha),
        ConstructKind::Fork => fork_layers(geo, old_count, color, alpha, extra_alpha),
        ConstructKind::Lattice => lattice_layers(geo, old_count, color, alpha, extra_alpha),
        ConstructKind::Tick => tick_layers(geo, color, alpha),
    }
}

fn arc_layers(geo: &Geo, color: u32, alpha: u32) -> Vec<(Path<Pixels>, u32)> {
    let span = geo.x1 - geo.x0;
    if span <= 0.0 {
        return Vec::new();
    }
    let xm = (geo.x0 + geo.x1) * 0.5;
    let mut path = PathBuilder::stroke(px(TERRAIN_STROKE));
    path.move_to(point(px(geo.x0), px(geo.floor)));
    path.curve_to(point(px(xm), px(geo.top)), point(px(geo.x0), px(geo.top)));
    path.curve_to(
        point(px(geo.x1), px(geo.floor)),
        point(px(geo.x1), px(geo.top)),
    );
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
) -> Vec<(Path<Pixels>, u32)> {
    let turns = geo.count.clamp(COIL_TURNS_MIN, COIL_TURNS_MAX);
    let span = geo.x1 - geo.x0;
    let rise = geo.floor - geo.top;
    if span <= 0.0 || rise <= 0.0 {
        return Vec::new();
    }
    let step = span / turns as f32;
    let mut layers = Vec::new();
    for turn in 0..turns {
        let from_x = geo.x0 + turn as f32 * step;
        let to_x = from_x + step;
        let from_y = geo.floor - rise * (turn as f32 / turns as f32);
        let to_y = geo.floor - rise * ((turn + 1) as f32 / turns as f32);
        let rx = step * 0.36 * (1.0 - 0.55 * turn as f32 / turns as f32);
        let mut path = PathBuilder::stroke(px(TERRAIN_STROKE));
        path.move_to(point(px(from_x), px(from_y)));
        path.arc_to(
            point(px(rx), px(step * 0.5)),
            px(0.0),
            false,
            turn % 2 == 1,
            point(px(to_x), px(to_y)),
        );
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
) -> Vec<(Path<Pixels>, u32)> {
    let span = geo.x1 - geo.x0;
    let rise = geo.floor - geo.top;
    if span <= 0.0 || rise <= 0.0 {
        return Vec::new();
    }
    let crest = geo.floor - rise * 0.45;
    let mut layers = Vec::new();
    let mut ridge = PathBuilder::stroke(px(TERRAIN_STROKE));
    ridge.move_to(point(px(geo.x0), px(crest)));
    ridge.line_to(point(px(geo.x1), px(crest)));
    if let Ok(path) = ridge.build() {
        layers.push((path, (color << 8) | alpha));
    }
    let arms = geo.count.max(1);
    for arm in 0..arms {
        let x = geo.x0 + (arm as f32 + 0.5) * span / arms as f32;
        let mut prong = PathBuilder::stroke(px(TERRAIN_STROKE));
        prong.move_to(point(px(x), px(crest)));
        prong.line_to(point(px(x), px(geo.top)));
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
) -> Vec<(Path<Pixels>, u32)> {
    let span = geo.x1 - geo.x0;
    if span <= 0.0 {
        return Vec::new();
    }
    let x0 = geo.x0 + span * 0.15;
    let x1 = geo.x1 - span * 0.15;
    let mut layers = Vec::new();
    let mut frame = PathBuilder::stroke(px(TERRAIN_STROKE));
    frame.move_to(point(px(x0), px(geo.top)));
    frame.line_to(point(px(x1), px(geo.top)));
    frame.line_to(point(px(x1), px(geo.floor)));
    frame.line_to(point(px(x0), px(geo.floor)));
    frame.line_to(point(px(x0), px(geo.top)));
    if let Ok(path) = frame.build() {
        layers.push((path, (color << 8) | alpha));
    }
    for column in [x0 + span * 0.30, x0 + span * 0.48] {
        let mut vertical = PathBuilder::stroke(px(TERRAIN_STROKE));
        vertical.move_to(point(px(column), px(geo.top)));
        vertical.line_to(point(px(column), px(geo.floor)));
        if let Ok(path) = vertical.build() {
            layers.push((path, (color << 8) | alpha));
        }
    }
    let rows = geo.count;
    if rows > 0 {
        let row_step = (geo.floor - geo.top) / (rows + 2) as f32;
        for row in 1..=rows {
            let y = geo.top + row as f32 * row_step;
            let mut line = PathBuilder::stroke(px(TERRAIN_STROKE));
            line.move_to(point(px(x0), px(y)));
            line.line_to(point(px(x1), px(y)));
            if let Ok(path) = line.build() {
                let part_alpha = if row <= old_count { alpha } else { extra_alpha };
                layers.push((path, (color << 8) | part_alpha));
            }
        }
    }
    layers
}

fn tick_layers(geo: &Geo, color: u32, alpha: u32) -> Vec<(Path<Pixels>, u32)> {
    let span = geo.x1 - geo.x0;
    let rise = geo.floor - geo.top;
    if span <= 0.0 || rise <= 0.0 {
        return Vec::new();
    }
    let xm = (geo.x0 + geo.x1) * 0.5;
    let half = (rise * 0.25).min(span * 0.5);
    let mut path = PathBuilder::stroke(px(TERRAIN_STROKE));
    path.move_to(point(px(xm - half), px(geo.floor)));
    path.line_to(point(px(xm), px(geo.top)));
    path.line_to(point(px(xm + half), px(geo.floor)));
    match path.build() {
        Ok(path) => vec![(path, (color << 8) | alpha)],
        Err(_) => Vec::new(),
    }
}

fn mark_count(mark: &TerrainMark) -> usize {
    count_for(mark.kind, mark.arms, mark.end - mark.start + 1)
}

fn old_count(mark: &TerrainMark, old: &TerrainGeo) -> usize {
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

fn band_span(start: usize, end: usize, rows: usize, width: f32) -> (f32, f32) {
    let rows = rows.max(1) as f32;
    min_span(
        start as f32 / rows * width,
        (end + 1) as f32 / rows * width,
        width,
    )
}

fn band_x(line: usize, rows: usize, width: f32) -> f32 {
    (line as f32 / rows.max(1) as f32 * width).clamp(0.0, width)
}

fn min_span(x0: f32, x1: f32, width: f32) -> (f32, f32) {
    if x1 - x0 >= MIN_SPAN_PX {
        return (x0, x1);
    }
    let x0 = x0.max(0.0);
    let x1 = x1.min(width);
    if x0 < MIN_SPAN_PX {
        return (0.0, MIN_SPAN_PX.min(width));
    }
    if width - x1 < MIN_SPAN_PX {
        return ((width - MIN_SPAN_PX).max(0.0), width);
    }
    let mid = (x0 + x1) * 0.5;
    (mid - MIN_SPAN_PX * 0.5, mid + MIN_SPAN_PX * 0.5)
}

fn depth_shift(depth: usize) -> f32 {
    (depth as f32 * DEPTH_STEP_PX).min(MAX_DEPTH_SHIFT_PX)
}

fn depth_dim(depth: usize) -> f32 {
    (1.0 - DEPTH_DIM * depth as f32).max(MIN_DEPTH_DIM)
}

fn glyph_height(kind: ConstructKind, depth: usize) -> f32 {
    kind_height(kind) + depth_shift(depth)
}

fn kind_height(kind: ConstructKind) -> f32 {
    match kind {
        ConstructKind::Arc => ARC_HEIGHT_PX,
        ConstructKind::Coil => COIL_HEIGHT_PX,
        ConstructKind::Fork => FORK_HEIGHT_PX,
        ConstructKind::Lattice => LATTICE_HEIGHT_PX,
        ConstructKind::Tick => TICK_HEIGHT_PX,
    }
}

fn breathe(alpha: u32, heat: HeatLevel, phase: f32) -> u32 {
    if heat == HeatLevel::Cool {
        return alpha;
    }
    let wave = 1.0 + PULSE_AMPLITUDE * (phase * std::f32::consts::TAU / PULSE_PERIOD_S).sin();
    ((alpha as f32) * wave).round().clamp(0.0, 255.0) as u32
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
        HeatLevel::Cool => ALPHA_COOL,
        HeatLevel::Warm => ALPHA_WARM,
        HeatLevel::Hot => ALPHA_HOT,
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
    use gpui::{point, size};

    fn spec(rows: usize, marks: Vec<TerrainMark>, deaths: Vec<TerrainDeath>) -> Rc<TerrainSpec> {
        Rc::new(TerrainSpec {
            seq: 1,
            morphing: false,
            rows,
            marks,
            deaths,
        })
    }

    fn mark(
        kind: ConstructKind,
        start: usize,
        end: usize,
        depth: usize,
        arms: usize,
        heat: HeatLevel,
        old: Option<TerrainGeo>,
    ) -> TerrainMark {
        TerrainMark {
            kind,
            start,
            end,
            depth,
            arms,
            heat,
            old,
        }
    }

    fn death(
        kind: ConstructKind,
        start: usize,
        end: usize,
        depth: usize,
        arms: usize,
        heat: HeatLevel,
    ) -> TerrainDeath {
        TerrainDeath {
            kind,
            start,
            end,
            depth,
            arms,
            heat,
            rows: 10,
        }
    }

    fn paint_layers(
        spec: &Rc<TerrainSpec>,
        progress: f32,
        width: f32,
        height: f32,
    ) -> Vec<(Path<Pixels>, u32)> {
        paint(
            spec,
            progress,
            0.0,
            Bounds::new(point(px(0.0), px(0.0)), size(px(width), px(height))),
        )
        .layers
    }

    #[test]
    fn offset_geo_shifts_geometry_into_window_space() {
        let shifted = offset_geo(
            Geo {
                x0: 10.0,
                x1: 40.0,
                top: 60.0,
                floor: 100.0,
                count: 3,
            },
            320.0,
            180.0,
        );
        assert_eq!((shifted.x0, shifted.x1), (330.0, 360.0));
        assert_eq!((shifted.top, shifted.floor), (240.0, 280.0));
        assert_eq!(shifted.count, 3);
        let same = offset_geo(
            Geo {
                x0: 10.0,
                x1: 40.0,
                top: 60.0,
                floor: 100.0,
                count: 3,
            },
            0.0,
            0.0,
        );
        assert_eq!((same.x0, same.x1), (10.0, 40.0));
        assert_eq!((same.top, same.floor), (60.0, 100.0));
    }

    #[test]
    fn heat_ramp_walks_cool_to_ember_to_white_hot() {
        assert_eq!(heat_color(HeatLevel::Cool), AURORA_COLD);
        assert_eq!(heat_color(HeatLevel::Warm), AURORA_EMBER);
        assert_eq!(heat_color(HeatLevel::Hot), STAR_HOT);
        assert!(heat_alpha(HeatLevel::Cool) < heat_alpha(HeatLevel::Warm));
        assert!(heat_alpha(HeatLevel::Warm) < heat_alpha(HeatLevel::Hot));
    }

    #[test]
    fn depth_shift_is_bounded_and_stacks_height() {
        assert_eq!(depth_shift(0), 0.0);
        assert_eq!(depth_shift(1), 8.0);
        assert_eq!(depth_shift(3), 24.0);
        assert_eq!(depth_shift(20), MAX_DEPTH_SHIFT_PX);
        assert!(
            glyph_height(ConstructKind::Arc, 3) > glyph_height(ConstructKind::Arc, 1),
            "deeper constructs rise higher"
        );
    }

    #[test]
    fn deeper_constructs_dim_toward_the_back() {
        assert_eq!(depth_dim(0), 1.0);
        assert_eq!(depth_dim(1), 0.9);
        assert_eq!(depth_dim(2), 0.8);
        assert_eq!(depth_dim(20), MIN_DEPTH_DIM);
        assert!(depth_dim(1) < depth_dim(0));
        assert!(depth_dim(5) < depth_dim(1));
    }

    #[test]
    fn band_span_maps_lines_to_the_full_width() {
        let (x0, x1) = band_span(0, 9, 10, 500.0);
        assert_eq!(x0, 0.0);
        assert_eq!(x1, 500.0, "the first and last lines span the pane");
        let (x0, x1) = band_span(4, 6, 10, 500.0);
        assert_eq!(x0, 200.0);
        assert_eq!(x1, 350.0);
        let (x0, x1) = band_span(0, 0, 1, 500.0);
        assert_eq!((x0, x1), (0.0, 500.0), "a single line fills the pane");
    }

    #[test]
    fn tiny_bands_grow_to_the_minimum_span() {
        let (x0, x1) = band_span(4, 4, 10, 500.0);
        assert!(x1 - x0 >= MIN_SPAN_PX, "a one-line band stays visible");
        let (x0, x1) = band_span(9, 9, 1000, 500.0);
        assert!(x1 - x0 >= MIN_SPAN_PX);
        let (x0, x1) = band_span(0, 0, 1000, 500.0);
        assert_eq!(
            (x0, x1),
            (0.0, MIN_SPAN_PX),
            "pinned to the left edge, the band grows right"
        );
        let (x0, x1) = band_span(999, 999, 1000, 500.0);
        assert_eq!(
            (x0, x1),
            (500.0 - MIN_SPAN_PX, 500.0),
            "pinned to the right edge, the band grows left"
        );
    }

    #[test]
    fn glyphs_never_escape_the_terrain_band() {
        let band = 578.0 * TERRAIN_HEIGHT_FRACTION;
        for kind in [
            ConstructKind::Arc,
            ConstructKind::Coil,
            ConstructKind::Fork,
            ConstructKind::Lattice,
            ConstructKind::Tick,
        ] {
            assert!(glyph_height(kind, 0) > 0.0);
            assert!(
                glyph_height(kind, usize::MAX) < band,
                "{kind:?} at any depth stays inside the bottom band"
            );
        }
        assert!(
            glyph_height(ConstructKind::Arc, 0) > glyph_height(ConstructKind::Tick, 0),
            "arcs are the tallest hills, ticks the shortest spikes"
        );
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

    #[test]
    fn breathe_only_moves_warm_and_hot_parts() {
        assert_eq!(breathe(0x66, HeatLevel::Cool, 0.0), 0x66);
        assert_eq!(breathe(0x66, HeatLevel::Cool, 1.0), 0x66);
        let peak = breathe(ALPHA_WARM, HeatLevel::Warm, PULSE_PERIOD_S / 4.0);
        assert!(peak > ALPHA_WARM, "warm breathes bright at the pulse peak");
        let trough = breathe(ALPHA_WARM, HeatLevel::Warm, PULSE_PERIOD_S * 0.75);
        assert!(trough < ALPHA_WARM, "warm breathes dim at the pulse trough");
        assert_eq!(
            breathe(ALPHA_WARM, HeatLevel::Warm, 0.0),
            ALPHA_WARM,
            "the pulse passes through the base"
        );
    }

    #[test]
    fn kind_glyphs_paint_their_discrete_layers() {
        let width = 500.0;
        let height = 115.0;
        let arc = paint_layers(
            &spec(
                10,
                vec![mark(ConstructKind::Arc, 2, 7, 0, 0, HeatLevel::Hot, None)],
                Vec::new(),
            ),
            1.0,
            width,
            height,
        );
        assert_eq!(arc.len(), 1, "an arc is a single hill cap");
        assert_eq!(arc[0].1 >> 8, STAR_HOT, "a hot cap paints white-hot");
        let coil = paint_layers(
            &spec(
                10,
                vec![mark(ConstructKind::Coil, 2, 6, 0, 0, HeatLevel::Warm, None)],
                Vec::new(),
            ),
            1.0,
            width,
            height,
        );
        assert_eq!(coil.len(), 5, "a five-line coil winds five turns");
        let fork = paint_layers(
            &spec(
                10,
                vec![mark(ConstructKind::Fork, 2, 8, 0, 3, HeatLevel::Warm, None)],
                Vec::new(),
            ),
            1.0,
            width,
            height,
        );
        assert_eq!(
            fork.len(),
            4,
            "a three-arm fork is a ridge plus three prongs"
        );
        let lattice = paint_layers(
            &spec(
                10,
                vec![mark(
                    ConstructKind::Lattice,
                    1,
                    4,
                    0,
                    0,
                    HeatLevel::Cool,
                    None,
                )],
                Vec::new(),
            ),
            1.0,
            width,
            height,
        );
        assert_eq!(
            lattice.len(),
            5,
            "a four-line lattice is a frame, two verticals, and two steps"
        );
        let tick = paint_layers(
            &spec(
                10,
                vec![mark(ConstructKind::Tick, 3, 3, 0, 0, HeatLevel::Cool, None)],
                Vec::new(),
            ),
            1.0,
            width,
            height,
        );
        assert_eq!(tick.len(), 1, "a tick is a single spike");
    }

    #[test]
    fn birth_draws_in_white_hot_and_cools_to_the_heat_ramp() {
        let spec = spec(
            10,
            vec![mark(ConstructKind::Arc, 2, 7, 0, 0, HeatLevel::Cool, None)],
            Vec::new(),
        );
        let fresh = paint_layers(&spec, 0.0, 500.0, 115.0);
        assert_eq!(fresh.len(), 1);
        let fresh_color = fresh[0].1;
        assert_eq!(fresh_color >> 8, STAR_HOT, "a birth ignites white-hot");
        assert_eq!(
            fresh_color & 0xff,
            BIRTH_ALPHA_PEAK,
            "a birth peaks at full brightness"
        );
        let settled = paint_layers(&spec, 1.0, 500.0, 115.0);
        assert_eq!(
            settled[0].1 >> 8,
            AURORA_COLD,
            "the birth cools onto the heat ramp"
        );
        assert!(
            settled[0].1 & 0xff < BIRTH_ALPHA_PEAK,
            "the cooled glyph dims to its ramp alpha"
        );
    }

    #[test]
    fn death_collapses_into_the_terrain_and_dissolves() {
        let spec = spec(
            10,
            Vec::new(),
            vec![death(ConstructKind::Arc, 2, 7, 0, 0, HeatLevel::Warm)],
        );
        let early = paint_layers(&spec, 0.0, 500.0, 115.0);
        assert_eq!(early.len(), 1);
        assert!(
            early[0].1 & 0xff > 0,
            "a death is still visible at the start"
        );
        let gone = paint_layers(&spec, 1.0, 500.0, 115.0);
        assert_eq!(
            gone[0].1 & 0xff,
            0,
            "a death fades to nothing by the end of the window"
        );
    }

    #[test]
    fn morph_warps_the_band_between_old_and_new_geometry() {
        let spec = spec(
            10,
            vec![mark(
                ConstructKind::Arc,
                5,
                9,
                0,
                0,
                HeatLevel::Warm,
                Some(TerrainGeo {
                    start: 0,
                    end: 4,
                    arms: 0,
                    rows: 10,
                }),
            )],
            Vec::new(),
        );
        let early = paint_layers(&spec, 0.0, 500.0, 115.0);
        let settled = paint_layers(&spec, 1.0, 500.0, 115.0);
        assert_eq!(early.len(), 1);
        assert_eq!(settled.len(), 1);
        assert_eq!(
            early[0].1, settled[0].1,
            "a morph keeps color and alpha, only the geometry warps"
        );
    }
}
