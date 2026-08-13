use std::rc::Rc;

use gpui::prelude::*;
use gpui::{canvas, point, px, rgba, Bounds, Canvas, Path, PathBuilder, Pixels};
use qol_diff::transition::{NewLineFate, TransitionPlan};
use qol_diff::waveform::{WavePoint, WAVE_DEEP_BLUE, WAVE_EMBER};

const AMPLITUDE_FRACTION: f32 = 0.38;
const TICK_MIN_PX: f32 = 2.0;
const TICK_PER_RIPPLE: f32 = 6.0;
const GHOST_DRIFT_PX: f32 = 10.0;
const STROKE_WIDTH: f32 = 2.0;
const STROKE_ALPHA: u32 = 0xe6;
const TICK_ALPHA: u32 = 0xcc;
const FILL_ALPHA_BASE: u32 = 0x10;
const FILL_ALPHA_VOLUME: u32 = 0x10;
const BASELINE_ALPHA: u32 = 0x60;
const PLAYHEAD_ALPHA: u32 = 0x90;

pub struct WaveMorph {
    pub old_points: Rc<Vec<WavePoint>>,
    pub plan: Rc<TransitionPlan>,
}

pub struct WavePaint {
    pub layers: Vec<(Path<Pixels>, u32)>,
}

struct WaveRenderer {
    points: Rc<Vec<WavePoint>>,
    morph: Option<Rc<WaveMorph>>,
    playhead: usize,
    progress: f32,
}

pub fn wave_element(
    points: Rc<Vec<WavePoint>>,
    morph: Option<Rc<WaveMorph>>,
    playhead: usize,
    progress: f32,
) -> Canvas<WavePaint> {
    let renderer = WaveRenderer {
        points,
        morph,
        playhead,
        progress,
    };
    canvas(
        move |bounds, _, _| renderer.geometry(bounds),
        |_, paint, window, _| {
            for (path, color) in paint.layers {
                window.paint_path(path, rgba(color));
            }
        },
    )
    .size_full()
}

impl WaveRenderer {
    fn geometry(&self, bounds: Bounds<Pixels>) -> WavePaint {
        let width = bounds.size.width.to_f64() as f32;
        let height = bounds.size.height.to_f64() as f32;
        let center = height / 2.0;
        let count = self.points.len();
        let max_disp = self
            .points
            .iter()
            .map(|point| point.displacement.abs())
            .fold(0.0f32, f32::max);
        let scale = if max_disp > 0.0 {
            height * AMPLITUDE_FRACTION / max_disp
        } else {
            0.0
        };
        let ys: Vec<f32> = self
            .points
            .iter()
            .map(|point| center - point.displacement * scale)
            .collect();
        let mut layers = Vec::new();
        layers.push((
            horizontal_path(0.0, width, center),
            (WAVE_DEEP_BLUE << 8) | BASELINE_ALPHA,
        ));
        if let Some(morph) = &self.morph {
            let old_ys: Vec<f32> = morph
                .old_points
                .iter()
                .map(|point| center - point.displacement * scale)
                .collect();
            if self.progress < 1.0 {
                let drift = GHOST_DRIFT_PX * self.progress;
                let drifted: Vec<f32> = old_ys.iter().map(|y| y + drift).collect();
                let fade = 1.0 - self.progress;
                let mut ghost = wave_layers(&morph.old_points, &drifted, width, center);
                for (_, color) in ghost.iter_mut() {
                    let alpha = ((*color & 0xff) as f32 * fade).round() as u32;
                    *color = (*color & 0xffffff00) | alpha.min(0xff);
                }
                layers.append(&mut ghost);
            }
            let mut blended = ys.clone();
            for (index, _) in self.points.iter().enumerate() {
                let from = match morph.plan.new.get(index) {
                    Some(NewLineFate::CarriedFrom(old_index))
                    | Some(NewLineFate::MorphedFrom(old_index)) => {
                        old_ys.get(*old_index).copied().unwrap_or(center)
                    }
                    _ => center,
                };
                blended[index] = from + (ys[index] - from) * self.progress;
            }
            layers.extend(wave_layers(&self.points, &blended, width, center));
        } else {
            layers.extend(wave_layers(&self.points, &ys, width, center));
        }
        if count > 1 {
            let playhead = self.playhead.min(count - 1);
            let x = wave_x(playhead, count, width);
            layers.push((
                vertical_path(x, 0.0, height),
                (WAVE_EMBER << 8) | PLAYHEAD_ALPHA,
            ));
        }
        WavePaint { layers }
    }
}

fn wave_layers(
    points: &[WavePoint],
    ys: &[f32],
    width: f32,
    center: f32,
) -> Vec<(Path<Pixels>, u32)> {
    let count = points.len();
    let mut layers = Vec::new();
    if count == 0 {
        return layers;
    }
    let max_amp = points
        .iter()
        .map(|point| point.amplitude)
        .fold(0.0f32, f32::max);
    let mut index = 0;
    while index < count {
        if points[index].amplitude <= 0.0 {
            index += 1;
            continue;
        }
        let mut end = index;
        while end + 1 < count && points[end + 1].amplitude > 0.0 {
            end += 1;
        }
        let peak = (index..=end)
            .max_by(|left, right| {
                points[*left]
                    .displacement
                    .abs()
                    .total_cmp(&points[*right].displacement.abs())
            })
            .unwrap_or(index);
        let mut path = PathBuilder::fill();
        path.move_to(point(px(wave_x(index, count, width)), px(center)));
        path.line_to(point(px(wave_x(index, count, width)), px(ys[index])));
        for (row, y) in ys.iter().enumerate().take(end + 1).skip(index + 1) {
            path.line_to(point(px(wave_x(row, count, width)), px(*y)));
        }
        path.line_to(point(px(wave_x(end, count, width)), px(center)));
        path.close();
        if let Ok(path) = path.build() {
            let volume = if max_amp > 0.0 {
                points[peak].amplitude / max_amp
            } else {
                0.0
            };
            let alpha = FILL_ALPHA_BASE + (FILL_ALPHA_VOLUME as f32 * volume).round() as u32;
            layers.push((path, (points[peak].color << 8) | alpha.min(0xff)));
        }
        index = end + 1;
    }
    index = 0;
    while index < count {
        let color = points[index].color;
        let mut end = index;
        while end + 1 < count && points[end + 1].color == color {
            end += 1;
        }
        let mut path = PathBuilder::stroke(px(STROKE_WIDTH));
        path.move_to(point(px(wave_x(index, count, width)), px(ys[index])));
        for (row, y) in ys.iter().enumerate().take(end + 1).skip(index + 1) {
            path.line_to(point(px(wave_x(row, count, width)), px(*y)));
        }
        if end + 1 < count {
            path.line_to(point(px(wave_x(end + 1, count, width)), px(ys[end])));
            path.line_to(point(px(wave_x(end + 1, count, width)), px(ys[end + 1])));
        }
        if let Ok(path) = path.build() {
            layers.push((path, (color << 8) | STROKE_ALPHA));
        }
        index = end + 1;
    }
    index = 0;
    while index < count {
        if points[index].amplitude <= 0.0 {
            index += 1;
            continue;
        }
        let color = points[index].color;
        let mut end = index;
        while end + 1 < count && points[end + 1].amplitude > 0.0 && points[end + 1].color == color {
            end += 1;
        }
        let mut path = PathBuilder::stroke(px(1.0));
        for row in index..=end {
            let length = TICK_MIN_PX + points[row].ripple * TICK_PER_RIPPLE;
            let upward = points[row].displacement >= 0.0;
            let x = wave_x(row, count, width);
            let from = ys[row];
            let to = if upward { from - length } else { from + length };
            path.move_to(point(px(x), px(from)));
            path.line_to(point(px(x), px(to)));
        }
        if let Ok(path) = path.build() {
            layers.push((path, (color << 8) | TICK_ALPHA));
        }
        index = end + 1;
    }
    layers
}

fn wave_x(row: usize, count: usize, width: f32) -> f32 {
    if count <= 1 {
        width / 2.0
    } else {
        row as f32 / (count - 1) as f32 * width
    }
}

fn horizontal_path(from: f32, to: f32, y: f32) -> Path<Pixels> {
    let mut path = PathBuilder::stroke(px(1.0));
    path.move_to(point(px(from), px(y)));
    path.line_to(point(px(to), px(y)));
    path.build().expect("two-point stroke path builds")
}

fn vertical_path(x: f32, from: f32, to: f32) -> Path<Pixels> {
    let mut path = PathBuilder::stroke(px(1.0));
    path.move_to(point(px(x), px(from)));
    path.line_to(point(px(x), px(to)));
    path.build().expect("two-point stroke path builds")
}
